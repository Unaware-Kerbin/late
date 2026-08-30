import type { OpenAiTool } from "./types.js";

export type McpToolDef = {
  name: string;
  description?: string;
  inputSchema?: Record<string, unknown>;
};

/**
 * Read-only / status tools Late runs without Approve.
 * Unknown names wait — do not treat get_/list_/recommend_ prefixes as safe.
 */
const MCP_APPROVE_FREE = new Set([
  "list_agents",
  "chat_list",
  "chat_get",
  "list_runs",
  "get_run",
  "list_allowed_dirs",
  "list_hardware",
  "list_local_models",
  "recommend_local_models",
  "vllm_status",
  "ollama_status",
  "llamacpp_status",
]);

export function mcpNeedsApprove(rawName: string): boolean {
  const n = stripMcpPrefix(rawName).toLowerCase();
  return !MCP_APPROVE_FREE.has(n);
}

export function mcpToolName(rawName: string): string {
  const n = String(rawName ?? "")
    .trim()
    .replace(/[^a-zA-Z0-9_-]/g, "_")
    .slice(0, 60);
  if (!n) return "mcp_tool";
  if (n.startsWith("mcp_")) return n;
  return `mcp_${n}`.slice(0, 64);
}

export function stripMcpPrefix(name: string): string {
  return name.startsWith("mcp_") ? name.slice(4) : name;
}

/** Fill start_vllm with Late's "use all GPUs" choice unless the tool call already set it. */
export function withStartVllmGpuArgs(
  toolName: string,
  args: Record<string, unknown>,
  useAllGpus: boolean,
): Record<string, unknown> {
  const n = stripMcpPrefix(toolName).toLowerCase();
  if (n !== "start_vllm") return args;
  if ("use_all_gpus" in args || "useAllGpus" in args) return args;
  return { ...args, use_all_gpus: useAllGpus };
}

export function mcpToOpenAiTool(def: McpToolDef): OpenAiTool {
  const schema =
    def.inputSchema && typeof def.inputSchema === "object"
      ? def.inputSchema
      : { type: "object", properties: {} };
  const gated = mcpNeedsApprove(def.name);
  const extra = gated
    ? " Late will wait for the operator to click Approve before this runs."
    : "";
  return {
    type: "function",
    function: {
      name: mcpToolName(def.name),
      description: `${def.description?.trim() || def.name} (MCP).${extra}`,
      parameters: schema,
    },
  };
}

export function splitMcpArgs(raw: string): string[] | { error: string } {
  const t = raw.trim();
  if (!t) return [];
  if (/[;|&`$<>\\]/.test(t)) {
    return { error: "MCP args cannot contain shell characters" };
  }
  return t.split(/\s+/).filter(Boolean);
}

export type McpLaunch = {
  command: string;
  args: string[];
  cwd: string;
  env: Record<string, string>;
};

export type McpSettings = {
  enabled: boolean;
  cwd: string;
  command: string;
  args: string;
  url: string;
};

export type McpTarget =
  | { mode: "http"; url: string }
  | ({ mode: "stdio" } & McpLaunch)
  | { error: string };

const SHELL_OR_CONTROL = /[\x00-\x1f\x7f;|&`$<>\\]/;

/** http(s) MCP address only. Empty is not an address. */
export function parseMcpHttpUrl(raw: string): string | { error: string } {
  const t = raw.trim();
  if (!t) return { error: "MCP address is empty." };
  if (SHELL_OR_CONTROL.test(t)) {
    return { error: "MCP address cannot contain shell or control characters." };
  }
  if (/^file:/i.test(t) || /^javascript:/i.test(t) || /^data:/i.test(t)) {
    return { error: "MCP address must be http:// or https:// (not a file path)." };
  }
  let u: URL;
  try {
    u = new URL(t);
  } catch {
    return { error: "MCP address is not a valid URL." };
  }
  if (u.protocol !== "http:" && u.protocol !== "https:") {
    return { error: "MCP address must be http:// or https:// (not a file path)." };
  }
  if (u.username || u.password) {
    return { error: "MCP address cannot include a username or password. Use the URL only." };
  }
  if (!u.hostname) {
    return { error: "MCP address needs a host (IP or name)." };
  }
  rewriteLoopbackHostname(u);
  const stripped = u.pathname.replace(/\/+$/, "") || "/";
  const lower = stripped.toLowerCase();
  if (lower === "/" || lower === "/mcp") {
    u.pathname = "/mcp";
    if (lower === "/") {
      u.search = "";
      u.hash = "";
    }
  } else if (lower.startsWith("/mcp/")) {
    u.pathname = `/mcp${stripped.slice(4)}`;
  }
  return u.toString();
}

/** HTML page (GUI `/`) is not Streamable HTTP. Late Settings URL is the source of truth. */
export const GUI_NOT_MCP =
  "That address returned the web UI, not Streamable HTTP MCP. Use the /mcp URL printed when you started the GUI or npm run mcp:http (the address in Late Settings).";


/** MCP HTTP binds 127.0.0.1. Prefer that over localhost / ::1 so IPv6 DNS does not miss it. */
function rewriteLoopbackHostname(u: URL): void {
  const host = u.hostname.replace(/^\[|\]$/g, "").toLowerCase();
  if (host === "localhost" || host === "localhost.localdomain" || host === "::1") {
    u.hostname = "127.0.0.1";
  }
}

/** Same-origin `/mcp` to try if the pasted path is empty or `/MCP` (already folded by parse). */
export function mcpHttpProbeUrls(raw: string): string[] | { error: string } {
  const parsed = parseMcpHttpUrl(raw);
  if (typeof parsed !== "string") return parsed;
  const mcp = new URL(parsed);
  mcp.pathname = "/mcp";
  mcp.search = "";
  mcp.hash = "";
  const alt = mcp.toString();
  if (parsed === alt) return [parsed];
  return [parsed, alt];
}

export type JsonRpcMsg = {
  jsonrpc?: string;
  id?: number | string;
  result?: unknown;
  error?: { message?: string; code?: number };
  method?: string;
};

/** Model ids for Late's MCP Agent dropdown. Chat always uses orchestrator auto-route. */
export const MCP_GUI_AGENTS = ["orchestrator"] as const;

/** Specialist / backend ids from list_agents JSON. Always includes orchestrator. */
export function parseMcpCatalogAgents(raw: string): { agents: string[]; specialists: string[] } {
  const agents = ["orchestrator"];
  const specialists: string[] = [];
  try {
    const data = JSON.parse(raw) as Record<string, unknown>;
    const push = (id: string, specialist: boolean) => {
      if (!id || agents.includes(id)) return;
      agents.push(id);
      if (specialist) specialists.push(id);
    };
    const specs = Array.isArray(data.specialists) ? data.specialists : [];
    for (const s of specs) {
      if (typeof s === "string") push(s, true);
      else if (s && typeof s === "object" && typeof (s as { id?: unknown }).id === "string") {
        push((s as { id: string }).id, true);
      }
    }
    const backs = Array.isArray(data.backends) ? data.backends : [];
    for (const b of backs) {
      if (typeof b === "string") push(b, false);
      else if (b && typeof b === "object" && typeof (b as { id?: unknown }).id === "string") {
        push((b as { id: string }).id, false);
      }
    }
  } catch {
    /* orchestrator only */
  }
  return { agents, specialists };
}

/** Parse SSE `data:` frames that carry JSON-RPC. */
export function parseSseJsonRpc(text: string): JsonRpcMsg[] {
  const out: JsonRpcMsg[] = [];
  let data: string[] = [];
  const flush = () => {
    if (!data.length) return;
    const raw = data.join("\n").trim();
    data = [];
    if (!raw) return;
    try {
      out.push(JSON.parse(raw) as JsonRpcMsg);
    } catch {
      /* skip non-JSON SSE frames */
    }
  };
  for (const line of text.split(/\r?\n/)) {
    if (line.startsWith("data:")) {
      data.push(line.slice(5).replace(/^\s/, ""));
    } else if (line === "") {
      flush();
    }
  }
  flush();
  return out;
}

/**
 * URL wins when non-empty (operator chose an address). Otherwise spawn the folder on this computer.
 */
export function resolveMcpTarget(
  opts: McpSettings,
  exists: (p: string) => boolean,
  joinPath: (...parts: string[]) => string,
): McpTarget {
  if (!opts.enabled) return { error: "MCP is off in Settings" };
  const url = opts.url.trim();
  if (url) {
    const parsed = parseMcpHttpUrl(url);
    if (typeof parsed !== "string") return parsed;
    return { mode: "http", url: parsed };
  }
  const launch = resolveMcpLaunch(opts, exists, joinPath);
  if ("error" in launch) return launch;
  return { mode: "stdio", ...launch };
}

export function resolveMcpLaunch(
  opts: { enabled: boolean; cwd: string; command: string; args: string },
  exists: (p: string) => boolean,
  joinPath: (...parts: string[]) => string,
): McpLaunch | { error: string } {
  if (!opts.enabled) return { error: "MCP is off in Settings" };
  const cwd = opts.cwd.trim();
  if (!cwd) {
    return {
      error: "Set the MCP folder (this computer) or an address (another box) in Settings.",
    };
  }
  const extra = splitMcpArgs(opts.args);
  if ("error" in extra) return extra;
  const cmd = opts.command.trim();
  const tsx = joinPath(cwd, "node_modules", ".bin", "tsx");
  const src = joinPath(cwd, "src", "index.ts");
  const dist = joinPath(cwd, "dist", "index.js");
  const yaml = joinPath(cwd, "agents.config.yaml");
  let command = cmd;
  let args = extra;
  if (!command) {
    if (exists(tsx) && exists(src)) {
      command = tsx;
      args = args.length ? args : [src];
    } else if (exists(dist)) {
      command = process.execPath;
      args = args.length ? args : [dist];
    } else {
      return {
        error:
          "Need tsx + src/index.ts or dist/index.js in that folder. Run npm install there, or set the command.",
      };
    }
  } else if (!args.length && exists(src)) {
    args = [src];
  }
  const env: Record<string, string> = { WORKSPACE_CWD: cwd };
  if (exists(yaml)) env.AGENT_ORCHESTRATOR_CONFIG = yaml;
  return { command, args, cwd, env };
}

/** Late RPC / API key names. MCP stdio must not inherit these from the sidecar. */
const MCP_STDIO_DROP = [
  "LATE_RPC_TOKEN",
  "LATE_SIDECAR_TOKEN",
  "LATE_DAEMON_HTTP",
  "LATE_DAEMON_WS",
  "CURSOR_API_KEY",
  "ANTHROPIC_API_KEY",
  "GEMINI_API_KEY",
  "GOOGLE_API_KEY",
  "AZURE_OPENAI_API_KEY",
  "OPENAI_API_KEY",
  "GROQ_API_KEY",
  "OPENROUTER_API_KEY",
];

/**
 * Copy PATH/HOME/etc. for MCP stdio. Drop Late RPC and provider keys so the child
 * cannot call the daemon or reuse Cloud AI credentials.
 */
export function mcpStdioEnv(
  parent: NodeJS.Dict<string>,
  extra: Record<string, string>,
): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {};
  for (const [k, v] of Object.entries(parent)) {
    if (v === undefined) continue;
    if (MCP_STDIO_DROP.includes(k)) continue;
    env[k] = v;
  }
  Object.assign(env, extra);
  for (const k of MCP_STDIO_DROP) delete env[k];
  return env;
}
