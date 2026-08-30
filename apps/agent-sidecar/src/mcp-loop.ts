import {
  callMcpTool,
  closeMcp,
  ensureMcpSession,
  listMcpOpenAiTools,
  mcpChatTarget,
  mcpSettingsFromDaemon,
} from "./mcp-client.js";
import {
  extractMcpAssistantText,
  isWriteAllowlistDump,
  MCP_CHAT_CAP_MS,
  MCP_CHAT_POLL_MS,
  MCP_IGNORED_CONTROL_DUMP,
  mcpChatSendArgs,
  mcpOperatorSend,
  mcpVisibleReply,
  mcpSafeErrorMessage,
  mcpThreadPollIdle,
  operatorWantsAllowlist,
  parseMcpLateTool,
  firstLiveSessionUuid,
  isLiveSessionUuid,
  preferMcpStagedTool,
  type McpSpeakerTurn,
  type ParsedMcpTool,
} from "./mcp-chat-format.js";
import { firstReachableMcpUrl, mcpDiscoverCandidates } from "./mcp-discover.js";
import { mcpToolName, parseMcpHttpUrl } from "./mcp-format.js";
import { probeMcpHttpEndpoint } from "./mcp-http.js";
import {
  MCP_OFF_MESSAGE,
  MCP_NO_CHAT_SEND,
  MCP_STAY_HEARTBEAT,
  shouldDiscoverLoopbackMcp,
  stayOnMcp,
} from "./mcp-stay.js";
import {
  extractPcapPaths,
  fetchLogoDataUrl,
  grantTempPcap,
  readOrchestratorGuiToken,
  releaseTempPcap,
  restOriginsFromMcpUrl,
} from "./orch-rest.js";
import { assertChatAllowed } from "./openai-loop.js";
import { executeTool, inferNamedStageTool, liveSessionContext, type ToolCtx } from "./tools.js";
import { MAX_ROUNDS, SYSTEM_PROMPT, type ChatMessage, type SseEvent } from "./types.js";

export {
  MCP_CONNECT_ATTEMPTS,
  MCP_FALLBACK_HEARTBEAT,
  MCP_NO_CHAT_SEND,
  MCP_OFF_MESSAGE,
  MCP_RETRY_MS,
  MCP_STAY_HEARTBEAT,
  MCP_UNREACHABLE_MESSAGE,
  decideMcpStay,
  isMcpConnectFailure,
  isMcpUnavailable,
  mcpStayUnreachableMessage,
  shouldDiscoverLoopbackMcp,
  shouldFallbackToLocalHelper,
  stayOnMcp,
} from "./mcp-stay.js";
export {
  extractMcpAssistantText,
  isWriteAllowlistDump,
  LATE_INVESTIGATION_TOOLS,
  MCP_CHAT_CAP_MS,
  MCP_CHAT_POLL_MS,
  MCP_CHAT_SEND_PIN,
  MCP_IGNORED_CONTROL_DUMP,
  mcpChatSendArgs,
  mcpIsolationPrefix,
  mcpJsonToolPreamble,
  mcpOperatorSend,
  mcpVisibleReply,
  mcpSafeErrorMessage,
  mcpThreadPollIdle,
  operatorWantsAllowlist,
  parseMcpLateTool,
  preferMcpStagedTool,
  firstLiveSessionUuid,
} from "./mcp-chat-format.js";
export { firstReachableMcpUrl, mcpDiscoverCandidates } from "./mcp-discover.js";
export { parseMcpCatalogAgents } from "./mcp-format.js";
export { extractPcapPaths } from "./orch-rest.js";

const MCP_CHAT_ROUND = new Set(["chat_send", "dispatch"]);
/** Staging write is the answer. Another chat_send would re-echo the same JSON until the round cap. */
export function mcpLateToolEndsTurn(name: string): boolean {
  return name === "propose_staged_artifact";
}
const threads = new Map<string, { threadId: string }>();

function clip(s: string, n = 4000): string {
  if (s.length <= n) return s;
  return `${s.slice(0, 1200)}\n…[truncated ${s.length - n} chars]…\n${s.slice(-(n - 1400))}`;
}

function executeName(parsed: ParsedMcpTool): string {
  if (parsed.late) return parsed.name;
  return mcpToolName(parsed.name);
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    const t = setTimeout(resolve, ms);
    t.unref?.();
  });
}

function mcpListedName(name: string): string {
  return name.replace(/^mcp_/, "");
}

async function listMcpToolNames(): Promise<string[]> {
  const tools = await listMcpOpenAiTools();
  return tools.map((t) => mcpListedName(t.function.name));
}

async function emitNewSpeakers(
  speakers: McpSpeakerTurn[],
  seen: Set<string>,
  emit: (e: SseEvent) => void,
  token: string,
  origins: string[],
): Promise<string> {
  let added = "";
  for (const s of speakers) {
    const key = `${s.speaker}\0${s.content}`;
    if (seen.has(key)) continue;
    seen.add(key);
    const logoDataUrl = s.logoUrl ? await fetchLogoDataUrl(s.logoUrl, token, origins) : undefined;
    emit({
      type: "speaker",
      speaker: s.speaker,
      label: s.label,
      nickname: s.nickname,
      logoUrl: s.logoUrl,
      logoDataUrl,
      content: s.content,
      skipped: s.skipped === true,
    });
    const line = `${s.label}: ${s.content}`;
    added += added ? `\n\n${line}` : line;
  }
  return added;
}

/** When debate JSON is missing but the operator asked for a playbook, still fill Staging on your computer. */
export function mcpStagedFallback(
  operatorTurn: string,
  parsed: ParsedMcpTool | null,
  live: string,
): ParsedMcpTool | null {
  const format = inferNamedStageTool(operatorTurn);
  if (!format) return parsed;
  if (parsed?.name === "propose_staged_artifact") return parsed;
  const sessionId = firstLiveSessionUuid(live) ?? "";
  return {
    name: "propose_staged_artifact",
    late: true,
    args: {
      format,
      intent: operatorTurn.replace(/\s+/g, " ").trim().slice(0, 200) || `configure ${format}`,
      ...(sessionId ? { session_id: sessionId } : {}),
    },
  };
}

export function bindLiveSessionId(parsed: ParsedMcpTool | null, live: string): ParsedMcpTool | null {
  if (!parsed?.late) return parsed;
  const sid = String(parsed.args.session_id ?? "").trim();
  if (sid && isLiveSessionUuid(sid)) return parsed;
  const liveId = firstLiveSessionUuid(live);
  if (!liveId) return parsed;
  return { ...parsed, args: { ...parsed.args, session_id: liveId } };
}

function stagedHasEditorBody(t: ParsedMcpTool | null): boolean {
  return Boolean(t && typeof t.args.body === "string" && t.args.body.trim());
}

/** Omit-body ansible still fills Staging when the operator asked for a playbook. Body-seed always fills. */
function shouldExecuteStaged(staged: ParsedMcpTool | null, operatorTurn: string): boolean {
  if (!staged) return false;
  if (stagedHasEditorBody(staged)) return true;
  if (inferNamedStageTool(operatorTurn)) return true;
  return /\b(?:write|create|generate|draft)\b.{0,40}\bconfig\b/i.test(operatorTurn);
}

/** Debate may emit propose_command plus a staged JSON with body — execute the draft that has editor text. */
export function parseExtractedTool(
  operatorTurn: string,
  extracted: ReturnType<typeof extractMcpAssistantText>,
  live: string,
): ParsedMcpTool | null {
  const opts = { lastLate: true as const, operatorTurn };
  const chunks = [
    extracted.text,
    extracted.pendingSummary ?? "",
    ...extracted.speakers.map((s) => s.content),
  ];
  const found: ParsedMcpTool[] = [];
  let best: ParsedMcpTool | null = null;
  for (const chunk of chunks) {
    if (!chunk?.trim()) continue;
    const parsed = parseMcpLateTool(chunk, opts);
    if (!parsed) continue;
    found.push(parsed);
    if (parsed.late) best = parsed;
  }
  const staged = preferMcpStagedTool(found, operatorTurn);
  if (shouldExecuteStaged(staged, operatorTurn)) return bindLiveSessionId(staged, live);
  const cmd = [...found].reverse().find((t) => t.late && t.name !== "propose_staged_artifact") ?? null;
  if (cmd) return bindLiveSessionId(mcpStagedFallback(operatorTurn, cmd, live), live);
  return bindLiveSessionId(mcpStagedFallback(operatorTurn, staged ?? best, live), live);
}

async function flushExtracted(
  operatorTurn: string,
  extracted: ReturnType<typeof extractMcpAssistantText>,
  seen: Set<string>,
  emit: (e: SseEvent) => void,
  token: string,
  origins: string[],
  live: string,
): Promise<{ assistantText: string; parsed: ParsedMcpTool | null; ignoredDump: boolean }> {
  const parsed = parseExtractedTool(operatorTurn, extracted, live);
  const visible = mcpVisibleReply(operatorTurn, extracted.text, parsed);
  const pending = extracted.pending
    ? `\n\n${extracted.pending} Approve that in the MCP program — Late cannot click it.`
    : "";
  const ignoredDump =
    !visible && isWriteAllowlistDump(extracted.text) && !operatorWantsAllowlist(operatorTurn);
  let assistantText = "";
  if (extracted.speakers.length > 0 && !ignoredDump) {
    assistantText = await emitNewSpeakers(extracted.speakers, seen, emit, token, origins);
    if (pending) {
      emit({ type: "delta", text: pending });
      assistantText += pending;
    }
  } else {
    const chunk = ignoredDump ? MCP_IGNORED_CONTROL_DUMP : `${visible}${pending}`;
    if (chunk) {
      emit({ type: "delta", text: chunk });
      assistantText = chunk;
    }
  }
  return { assistantText, parsed, ignoredDump };
}

async function pollMcpThread(opts: {
  threadId: string;
  operatorTurn: string;
  emit: (e: SseEvent) => void;
  signal: AbortSignal;
  token: string;
  origins: string[];
  seen: Set<string>;
}): Promise<ReturnType<typeof extractMcpAssistantText>> {
  const started = Date.now();
  let last = extractMcpAssistantText("{}");
  while (Date.now() - started < MCP_CHAT_CAP_MS) {
    if (opts.signal.aborted) throw new Error("stopped");
    let getRaw: string;
    try {
      getRaw = await callMcpTool("chat_get", { thread_id: opts.threadId }, 8_000);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      const recovered = extractMcpAssistantText(msg);
      if (recovered.threadId || recovered.speakers.length > 0) {
        last = recovered;
        await emitNewSpeakers(last.speakers, opts.seen, opts.emit, opts.token, opts.origins);
        if (mcpThreadPollIdle(last)) return last;
        await sleep(MCP_CHAT_POLL_MS);
        continue;
      }
      throw new Error(mcpSafeErrorMessage(msg));
    }
    last = extractMcpAssistantText(getRaw);
    await emitNewSpeakers(last.speakers, opts.seen, opts.emit, opts.token, opts.origins);
    if (last.thinking.length) {
      const names = last.thinking.map((t) => t.label).join(", ");
      const secs = Math.max(1, Math.round((Date.now() - started) / 1000));
      opts.emit({ type: "heartbeat", message: `${names} thinking… ${secs}s` });
    }
    const idle = mcpThreadPollIdle(last);
    if (idle) return last;
    await sleep(MCP_CHAT_POLL_MS);
  }
  opts.emit({ type: "heartbeat", message: "MCP debate cap — showing who finished." });
  return last;
}

export async function runMcpChat(opts: {
  messages: ChatMessage[];
  model?: string;
  conversationId: string;
  emit: (e: SseEvent) => void;
  signal: AbortSignal;
  /** Override Settings URL (used after loopback /mcp discovery). */
  mcpUrl?: string;
}): Promise<string> {
  const override = opts.mcpUrl?.trim();
  const parsedOverride = override ? parseMcpHttpUrl(override) : undefined;
  if (parsedOverride && typeof parsedOverride !== "string") throw new Error(parsedOverride.error);
  const target = parsedOverride
    ? { mode: "http" as const, url: parsedOverride }
    : await mcpChatTarget();
  if ("error" in target) throw new Error(target.error);
  if (target.mode === "http") {
    const parsed = parseMcpHttpUrl(target.url);
    if (typeof parsed !== "string") throw new Error(parsed.error);
    await assertChatAllowed("mcp", parsed);
  } else {
    await assertChatAllowed("mcp");
  }
  await stayOnMcp({
    run: () => ensureMcpSession(true, typeof parsedOverride === "string" ? parsedOverride : undefined),
    emit: opts.emit,
    signal: opts.signal,
    reset: closeMcp,
  });

  const lastUser = [...opts.messages].reverse().find((m) => m.role === "user");
  const operatorTurn = (lastUser?.content ?? "").trim() || "Continue from the untrusted device output.";
  const live = await liveSessionContext();
  const ctx: ToolCtx = {
    conversationId: opts.conversationId,
    emit: opts.emit,
    signal: opts.signal,
    operatorText: operatorTurn,
  };
  const stored = threads.get(opts.conversationId);
  const listed = await listMcpToolNames();
  if (!listed.includes("chat_send")) {
    throw new Error(MCP_NO_CHAT_SEND);
  }
  const canPoll = listed.includes("chat_get");

  let assistantText = "";
  let deviceBlock = live;
  let extra = "";
  let threadId = stored?.threadId || undefined;
  const cfg = await mcpSettingsFromDaemon();
  const restCtx = {
    token: readOrchestratorGuiToken(target.mode === "stdio" ? target.cwd : cfg.cwd),
    origins: restOriginsFromMcpUrl(target.mode === "http" ? target.url : cfg.url),
  };
  const pcapPaths = extractPcapPaths(operatorTurn);
  const granted: string[] = [];
  for (const p of pcapPaths) {
    const g = await grantTempPcap(p, restCtx);
    if (g.ok && g.path) granted.push(g.path);
    else if (g.error) {
      extra = [extra, `Temp pcap grant failed for ${p}: ${g.error}`].filter(Boolean).join("\n");
    }
  }

  try {
    for (let round = 1; round <= MAX_ROUNDS; round++) {
      if (opts.signal.aborted) throw new Error("stopped");
      opts.emit({ type: "round", n: round, max: MAX_ROUNDS });
      const message = mcpOperatorSend(
        SYSTEM_PROMPT,
        deviceBlock,
        round === 1 ? operatorTurn : "Continue from that untrusted Late tool output.",
        extra,
      );
      extra = "";
      const seen = new Set<string>();
      const beat = setInterval(() => {
        opts.emit({ type: "heartbeat", message: "MCP is thinking…" });
      }, 4000);
      let extracted: ReturnType<typeof extractMcpAssistantText> = extractMcpAssistantText("");
      try {
        const wait = !canPoll;
        const sendTimeout = canPoll ? 15_000 : MCP_CHAT_CAP_MS;
        try {
          const raw = await callMcpTool("chat_send", mcpChatSendArgs(message, threadId, wait), sendTimeout);
          extracted = extractMcpAssistantText(raw);
        } catch (err) {
          const msg = err instanceof Error ? err.message : String(err);
          const recovered = extractMcpAssistantText(msg);
          if (recovered.threadId || recovered.speakers.length > 0) {
            extracted = recovered;
          } else {
            throw new Error(mcpSafeErrorMessage(msg));
          }
        }
        if (extracted.threadId) {
          threadId = extracted.threadId;
          threads.set(opts.conversationId, { threadId });
        }
        const dumpEarly =
          isWriteAllowlistDump(extracted.text) && !operatorWantsAllowlist(operatorTurn);
        if (!dumpEarly) {
          await emitNewSpeakers(extracted.speakers, seen, opts.emit, restCtx.token, restCtx.origins);
          if (canPoll && threadId && (extracted.busy === true || extracted.speakers.length === 0)) {
            extracted = await pollMcpThread({
              threadId,
              operatorTurn,
              emit: opts.emit,
              signal: opts.signal,
              token: restCtx.token,
              origins: restCtx.origins,
              seen,
            });
          }
        }
      } finally {
        clearInterval(beat);
      }
      const flushed = await flushExtracted(
        operatorTurn,
        extracted,
        seen,
        opts.emit,
        restCtx.token,
        restCtx.origins,
        deviceBlock,
      );
      if (flushed.assistantText) {
        assistantText += assistantText && !flushed.assistantText.startsWith("\n") ? `\n${flushed.assistantText}` : flushed.assistantText;
      }
      if (!flushed.parsed?.late) {
        if (flushed.ignoredDump) return assistantText || MCP_IGNORED_CONTROL_DUMP;
        return assistantText || extracted.text;
      }

      const name = executeName(flushed.parsed);
      const result = clip(await executeTool(name, JSON.stringify(flushed.parsed.args), ctx));
      if (mcpLateToolEndsTurn(flushed.parsed.name)) return assistantText;
      deviceBlock = `BEGIN UNTRUSTED DEVICE OUTPUT\n${result}\nEND UNTRUSTED DEVICE OUTPUT`;
      extra = MCP_CHAT_ROUND.has(flushed.parsed.name)
        ? "A previous MCP chat_send/dispatch is waiting on Late Approve. Do not invent another."
        : "If that untrusted output answers the question, reply with the answer ONLY. Do not propose another show.";
    }
    throw new Error(`agent hit the ${MAX_ROUNDS}-round cap`);
  } finally {
    for (const p of granted) {
      await releaseTempPcap(p, restCtx);
    }
  }
}

export type McpChatFn = (opts: {
  messages: ChatMessage[];
  model?: string;
  conversationId: string;
  emit: (e: SseEvent) => void;
  signal: AbortSignal;
}) => Promise<string>;

async function discoverAndRunMcp(opts: {
  messages: ChatMessage[];
  model?: string;
  conversationId: string;
  emit: (e: SseEvent) => void;
  signal: AbortSignal;
}): Promise<string | undefined> {
  const cfg = await mcpSettingsFromDaemon();
  const skip = new Set<string>();
  if (cfg.url.trim()) {
    const parsed = parseMcpHttpUrl(cfg.url);
    if (typeof parsed === "string") skip.add(parsed);
  }
  const found = await firstReachableMcpUrl(
    mcpDiscoverCandidates({ settingsUrl: cfg.url, mcpCwd: cfg.cwd }),
    (url) => probeMcpHttpEndpoint(url),
    skip,
  );
  if (!found) return undefined;
  opts.emit({ type: "heartbeat", message: `Using MCP at ${found}` });
  return runMcpChat({ ...opts, mcpUrl: found });
}

/**
 * Agent = MCP: Settings URL, then other loopback /mcp on connect refuse only
 * (GUI /mcp, dedicated mcp:http). Never Late's vLLM on :8000.
 * Live HTTP errors stay on MCP.
 */
export async function runMcpChatOrFallback(
  opts: {
    messages: ChatMessage[];
    model?: string;
    conversationId: string;
    emit: (e: SseEvent) => void;
    signal: AbortSignal;
  },
  deps?: {
    mcpChat?: McpChatFn;
    /** Ignored. Agent=MCP does not call Late vLLM. */
    localChat?: McpChatFn;
    mcpEnabled?: boolean;
    discoverChat?: () => Promise<string | undefined>;
  },
): Promise<string> {
  const mcpChat = deps?.mcpChat ?? runMcpChat;
  const discover =
    deps?.discoverChat ?? (deps?.mcpChat ? async () => undefined : () => discoverAndRunMcp(opts));
  let enabled = deps?.mcpEnabled;
  if (enabled === undefined) {
    try {
      enabled = (await mcpSettingsFromDaemon()).enabled;
    } catch {
      enabled = false;
    }
  }
  if (!enabled) throw new Error(MCP_OFF_MESSAGE);
  try {
    return await mcpChat(opts);
  } catch (err) {
    if (opts.signal.aborted) throw err;
    const msg = err instanceof Error ? err.message : String(err);
    if (!shouldDiscoverLoopbackMcp(msg)) throw err;
    try {
      const discovered = await discover();
      if (discovered !== undefined) return discovered;
    } catch (discoveredErr) {
      if (opts.signal.aborted) throw discoveredErr;
      throw discoveredErr;
    }
    opts.emit({ type: "heartbeat", message: MCP_STAY_HEARTBEAT });
    throw err;
  }
}
