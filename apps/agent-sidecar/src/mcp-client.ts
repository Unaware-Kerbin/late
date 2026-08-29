import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { createInterface } from "node:readline";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { classifyChatBase, parseExtraHosts } from "./chat-target.js";
import { daemon } from "./daemon.js";
import { firstReachableMcpUrl, mcpDiscoverCandidates } from "./mcp-discover.js";
import {
  MCP_GUI_AGENTS,
  mcpToOpenAiTool,
  parseMcpHttpUrl,
  resolveMcpTarget,
  stripMcpPrefix,
  withStartVllmGpuArgs,
  type McpSettings,
  type McpToolDef,
} from "./mcp-format.js";
import { recoverMcpThreadDump, sanitizeMcpStopMessage } from "./mcp-chat-format.js";
import {
  mcpHttpRpc,
  openMcpHttpSession,
  probeMcpHttpEndpoint,
  type JsonRpc,
  type McpHttpSession,
} from "./mcp-http.js";
import { isMcpConnectFailure, shouldReuseLiveMcpHttp } from "./mcp-stay.js";
import type { OpenAiTool } from "./types.js";

type StdioSession = {
  kind: "stdio";
  key: string;
  child: ChildProcessWithoutNullStreams;
  pending: Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>;
  nextId: number;
};

type HttpLive = {
  kind: "http";
  key: string;
  sess: McpHttpSession;
};

type Live = StdioSession | HttpLive;

let session: Live | null = null;
let lastProbe: { at: number; value: McpProbe } | null = null;
/** Settings URL at the time the live HTTP session was opened (may differ after discover). */
let openedForSettingsUrl = "";

type McpProbe = {
  ok: boolean;
  enabled: boolean;
  tools: string[];
  message: string;
};

function settingFlag(s: Record<string, unknown>, ...keys: string[]): boolean {
  for (const k of keys) {
    const v = s[k];
    if (typeof v === "boolean") return v;
  }
  return false;
}

function settingStr(s: Record<string, unknown>, ...keys: string[]): string {
  for (const k of keys) {
    const v = s[k];
    if (typeof v === "string" && v.trim()) return v.trim();
  }
  return "";
}

async function loadSettings(): Promise<Record<string, unknown>> {
  try {
    const value = await daemon.call<Record<string, unknown>>("settings.get");
    if (value && typeof value === "object") return value;
  } catch {
    /* MCP stays off if the daemon is down */
  }
  return {};
}

export async function mcpSettingsFromDaemon(): Promise<McpSettings> {
  const s = await loadSettings();
  return {
    enabled: settingFlag(s, "mcp_enabled", "mcpEnabled"),
    cwd: settingStr(s, "mcp_cwd", "mcpCwd"),
    command: settingStr(s, "mcp_command", "mcpCommand"),
    args: settingStr(s, "mcp_args", "mcpArgs"),
    url: settingStr(s, "mcp_url", "mcpUrl", "mcp_base_url", "mcpBaseUrl"),
  };
}

function writeLine(child: ChildProcessWithoutNullStreams, obj: unknown) {
  child.stdin.write(`${JSON.stringify(obj)}\n`);
}

function attachReader(sess: StdioSession) {
  const rl = createInterface({ input: sess.child.stdout });
  rl.on("line", (line) => {
    const t = line.trim();
    if (!t) return;
    let msg: JsonRpc;
    try {
      msg = JSON.parse(t) as JsonRpc;
    } catch {
      return;
    }
    if (msg.id === undefined || msg.id === null) return;
    const id = Number(msg.id);
    const wait = sess.pending.get(id);
    if (!wait) return;
    sess.pending.delete(id);
    if (msg.error) {
      wait.reject(new Error(msg.error.message || `MCP error ${msg.error.code ?? ""}`.trim()));
      return;
    }
    wait.resolve(msg.result);
  });
  sess.child.stderr.on("data", () => {
    /* MCP logs on stderr; do not echo (may include paths). */
  });
  sess.child.on("exit", () => {
    if (session === sess) session = null;
    for (const wait of sess.pending.values()) {
      wait.reject(new Error("MCP server exited"));
    }
    sess.pending.clear();
  });
}

function rpcStdio(sess: StdioSession, method: string, params?: unknown, timeoutMs = 8000): Promise<unknown> {
  const id = sess.nextId++;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      sess.pending.delete(id);
      reject(new Error(`MCP ${method} timed out`));
    }, timeoutMs);
    sess.pending.set(id, {
      resolve: (v) => {
        clearTimeout(timer);
        resolve(v);
      },
      reject: (e) => {
        clearTimeout(timer);
        reject(e);
      },
    });
    writeLine(sess.child, { jsonrpc: "2.0", id, method, params: params ?? {} });
  });
}

async function handshakeStdio(sess: StdioSession) {
  await rpcStdio(sess, "initialize", {
    protocolVersion: "2025-03-26",
    capabilities: { tools: {} },
    clientInfo: { name: "late", version: "0.1.6" },
  });
  writeLine(sess.child, { jsonrpc: "2.0", method: "notifications/initialized" });
}

async function assertMcpHttp(url: string, s: Record<string, unknown>): Promise<void> {
  const parsed = parseMcpHttpUrl(url);
  if (typeof parsed !== "string") throw new Error(parsed.error);
  const extra = parseExtraHosts(s.private_inference_hosts ?? s.privateInferenceHosts);
  const egress = await classifyChatBase(parsed, extra);
  void egress;
  return;
}

async function rpcLive(live: Live, method: string, params?: unknown, timeoutMs = 8000): Promise<unknown> {
  if (live.kind === "http") return mcpHttpRpc(live.sess, method, params, timeoutMs);
  return rpcStdio(live, method, params, timeoutMs);
}

export async function ensureMcpSession(force = false, overrideUrl?: string): Promise<void> {
  await ensureSession(overrideUrl, force);
}

async function ensureSession(overrideUrl?: string, force = false): Promise<Live> {
  const cfg = await mcpSettingsFromDaemon();
  const settingsUrl = cfg.url.trim();
  const url = (overrideUrl ?? settingsUrl).trim();
  if (
    session?.kind === "http" &&
    shouldReuseLiveMcpHttp({
      hasHttpSession: true,
      sessionKey: session.key,
      overrideUrl,
      settingsUrl,
      openedForSettingsUrl,
    })
  ) {
    return session;
  }
  if (session && !overrideUrl && !url && session.kind === "stdio") return session;
  const target = resolveMcpTarget(
    { ...cfg, enabled: force || cfg.enabled || Boolean(url), url },
    existsSync,
    join,
  );
  if ("error" in target) throw new Error(target.error);
  if (target.mode === "http") {
    const s = await loadSettings();
    await assertMcpHttp(target.url, s);
    const key = `http\0${target.url}`;
    if (session && session.kind === "http" && session.key === key) {
      openedForSettingsUrl = settingsUrl;
      return session;
    }
    closeMcp();
    const sess = await openMcpHttpSession(target.url);
    const live: HttpLive = { kind: "http", key, sess };
    session = live;
    openedForSettingsUrl = settingsUrl;
    return live;
  }
  const key = `${target.command}\0${target.args.join("\0")}\0${target.cwd}`;
  if (session && session.kind === "stdio" && session.key === key) return session;
  closeMcp();
  const child = spawn(target.command, target.args, {
    cwd: target.cwd,
    env: { ...process.env, ...target.env },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const sess: StdioSession = { kind: "stdio", key, child, pending: new Map(), nextId: 1 };
  attachReader(sess);
  session = sess;
  openedForSettingsUrl = "";
  try {
    await handshakeStdio(sess);
  } catch (err) {
    closeMcp();
    throw err;
  }
  return sess;
}

export function closeMcp() {
  lastProbe = null;
  openedForSettingsUrl = "";
  if (!session) return;
  const s = session;
  session = null;
  if (s.kind === "http") {
    return;
  }
  try {
    s.child.stdin.end();
  } catch {
    /* ignore */
  }
  s.child.kill("SIGTERM");
}

export async function listMcpOpenAiTools(): Promise<OpenAiTool[]> {
  const cfg = await mcpSettingsFromDaemon();
  if (!cfg.enabled) return [];
  const live = await ensureSession();
  const result = (await rpcLive(live, "tools/list")) as { tools?: McpToolDef[] };
  const tools = Array.isArray(result?.tools) ? result.tools : [];
  return tools.filter((t) => t?.name).map((t) => mcpToOpenAiTool(t));
}

export async function callMcpTool(
  lateName: string,
  args: Record<string, unknown>,
  timeoutMs = 60_000,
): Promise<string> {
  const live = await ensureSession();
  const name = stripMcpPrefix(lateName);
  const s = await loadSettings();
  const useAll =
    typeof s.useAllGpus === "boolean"
      ? s.useAllGpus
      : typeof s.use_all_gpus === "boolean"
        ? s.use_all_gpus
        : true;
  const forwarded = withStartVllmGpuArgs(name, args, useAll);
  const result = (await rpcLive(live, "tools/call", { name, arguments: forwarded }, timeoutMs)) as {
    content?: { type?: string; text?: string }[];
    isError?: boolean;
  };
  const text = (result?.content ?? [])
    .map((c) => (typeof c.text === "string" ? c.text : ""))
    .filter(Boolean)
    .join("\n");
  if (result?.isError) {
    const recovered = recoverMcpThreadDump(text);
    if (recovered) return text;
    throw new Error(sanitizeMcpStopMessage(text || "MCP tool failed"));
  }
  return text || JSON.stringify(result ?? {});
}

async function probeHttpUrl(url: string): Promise<McpProbe> {
  const s = await loadSettings();
  try {
    await assertMcpHttp(url, s);
    const probed = await probeMcpHttpEndpoint(url);
    if (!probed.ok) {
      return { ok: false, enabled: true, tools: [], message: probed.message };
    }
    const names = probed.tools.map((n) => mcpToOpenAiTool({ name: n }).function.name);
    return {
      ok: true,
      enabled: true,
      tools: names,
      message: `MCP online · ${names.length} tool${names.length === 1 ? "" : "s"} at ${probed.url}.`,
    };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    return { ok: false, enabled: true, tools: [], message };
  }
}

async function discoverReachableHttp(cfg: { url: string; cwd: string }, skipUrl?: string): Promise<string | undefined> {
  const skip = new Set<string>();
  if (skipUrl?.trim()) {
    const parsed = parseMcpHttpUrl(skipUrl);
    if (typeof parsed === "string") skip.add(parsed);
  }
  return firstReachableMcpUrl(
    mcpDiscoverCandidates({ settingsUrl: cfg.url, mcpCwd: cfg.cwd }),
    (candidate) => probeMcpHttpEndpoint(candidate),
    skip,
  );
}

async function probeMcpFresh(overrideUrl?: string): Promise<McpProbe> {
  const cfg = await mcpSettingsFromDaemon();
  const url = (overrideUrl ?? cfg.url).trim();
  if (!overrideUrl && !cfg.enabled) {
    return { ok: true, enabled: false, tools: [], message: "MCP is off. The helper still works." };
  }
  if (url) {
    const first = await probeHttpUrl(url);
    if (first.ok || overrideUrl) return first;
    if (!isMcpConnectFailure(first.message)) return first;
    const found = await discoverReachableHttp(cfg, url);
    if (found) return probeHttpUrl(found);
    return first;
  }
  const advertised = await discoverReachableHttp(cfg);
  if (advertised) return probeHttpUrl(advertised);
  try {
    const tools = await listMcpOpenAiTools();
    const names = tools.map((t) => t.function.name);
    return {
      ok: true,
      enabled: true,
      tools: names,
      message: `MCP online · ${names.length} tool${names.length === 1 ? "" : "s"} on this computer.`,
    };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    return { ok: false, enabled: true, tools: [], message };
  }
}

export async function probeMcp(force = false, overrideUrl?: string): Promise<McpProbe> {
  if (!force && !overrideUrl && lastProbe?.value.ok && Date.now() - lastProbe.at < 10_000) {
    return lastProbe.value;
  }
  const value = await probeMcpFresh(overrideUrl);
  if (!overrideUrl && value.ok) lastProbe = { at: Date.now(), value };
  else if (!overrideUrl && !value.ok) lastProbe = null;
  return value;
}

export type McpChatTarget =
  | { mode: "http"; url: string }
  | { mode: "stdio"; cwd: string }
  | { error: string };

/** Where MCP chat will go. Does not start that program. */
export async function mcpChatTarget(): Promise<McpChatTarget> {
  const cfg = await mcpSettingsFromDaemon();
  const target = resolveMcpTarget(
    { ...cfg, enabled: true, url: cfg.url },
    existsSync,
    join,
  );
  if ("error" in target) {
    return {
      error:
        target.error === "MCP is off in Settings"
          ? "Pick This computer or an HTTP address for MCP. Use the /mcp URL printed when you started the GUI or npm run mcp:http (Late Settings is the source of truth)."
          : target.error,
    };
  }
  if (target.mode === "http") return { mode: "http", url: target.url };
  return { mode: "stdio", cwd: target.cwd };
}

export type McpChatMeta = { base: string; egress: "loopback" | "private" | "cloud" };

export async function mcpTargetMeta(): Promise<McpChatMeta> {
  const t = await mcpChatTarget();
  if ("error" in t) return { base: "", egress: "loopback" };
  if (t.mode === "stdio") return { base: "folder on this computer", egress: "loopback" };
  const s = await loadSettings();
  const extra = parseExtraHosts(s.private_inference_hosts ?? s.privateInferenceHosts);
  const parsed = parseMcpHttpUrl(t.url);
  const url = typeof parsed === "string" ? parsed : t.url;
  const egress = await classifyChatBase(url, extra);
  return { base: url, egress };
}

/** Late's MCP model menu is orchestrator only (auto-route via chat_send). */
export async function listMcpAgents(): Promise<string[]> {
  return [...MCP_GUI_AGENTS];
}
