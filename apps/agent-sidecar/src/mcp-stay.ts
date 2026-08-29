import type { SseEvent } from "./types.js";
import { parseMcpHttpUrl } from "./mcp-format.js";

export const MCP_CONNECT_ATTEMPTS = 3;
export const MCP_RETRY_MS = 400;

/** Short heartbeat when the Settings MCP URL refused TCP. */
export const MCP_UNREACHABLE_MESSAGE = "MCP unreachable";

/** Agent=MCP stays on MCP. It does not switch to Late's vLLM on :8000. */
export const MCP_STAY_HEARTBEAT = "Staying on MCP — not using Late vLLM";

/** @deprecated Use MCP_STAY_HEARTBEAT. Kept so old tests that imported the name still typecheck during edit. */
export const MCP_FALLBACK_HEARTBEAT = MCP_STAY_HEARTBEAT;

export const MCP_OFF_MESSAGE =
  "MCP is off in Settings. Paste the /mcp URL printed by the GUI (Copy MCP URL) or npm run mcp:http. Agent=MCP does not use Late's vLLM on :8000.";

/** Agent=MCP needs chat_send. Extra mcp_* tools still work on other backends. */
export const MCP_NO_CHAT_SEND =
  "MCP has no chat_send tool. Agent=MCP needs chat_send; extra tools still work on other backends. Late will not switch to vLLM on :8000.";

export function mcpStayUnreachableMessage(detail = ""): string {
  const clipped = detail.trim().split(/\n/)[0]?.trim().slice(0, 180) ?? "";
  const extra = clipped && !/MCP unreachable|MCP is not reachable/i.test(clipped) ? ` Last error: ${clipped}` : "";
  return `${MCP_UNREACHABLE_MESSAGE}. Paste the printed /mcp URL in Settings. Late will not start that program and will not switch to local vLLM.${extra}`;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    const t = setTimeout(resolve, ms);
    t.unref?.();
  });
}

/**
 * True only when the MCP TCP connection itself failed.
 * HTTP 4xx/5xx (including 400) are live server replies — show that body; do not call this “unreachable”.
 */
export function isMcpConnectFailure(message: string): boolean {
  if (/vLLM is not reachable|127\.0\.0\.1:8000/i.test(message)) return false;
  if (/MCP HTTP\s+[1-5]\d\d/i.test(message)) return false;
  return /MCP unreachable|MCP is not reachable|ECONNREFUSED|ECONNRESET|UND_ERR_CONNECT_TIMEOUT|UND_ERR_SOCKET|fetch failed|MCP server exited/i.test(
    message,
  );
}

/**
 * True when Agent=MCP should try another loopback /mcp (not Late vLLM).
 * Live HTTP 4xx/5xx stay on this MCP URL.
 */
export function shouldDiscoverLoopbackMcp(message: string): boolean {
  if (/MCP HTTP\s+[1-5]\d\d/i.test(message)) return false;
  if (/MCP chat_send timed out|no chat_send/i.test(message)) return false;
  if (/MCP is off/i.test(message)) return false;
  if (/Set the MCP folder|Need tsx \+|Pick This computer/i.test(message)) return true;
  return isMcpConnectFailure(message);
}

/** @deprecated Agent=MCP does not fall back to Late vLLM. Use shouldDiscoverLoopbackMcp. */
export function shouldFallbackToLocalHelper(message: string): boolean {
  return shouldDiscoverLoopbackMcp(message);
}

export function isMcpUnavailable(message: string): boolean {
  return isMcpConnectFailure(message);
}

/** Keep a discovered GUI /mcp session while Settings still has the dead URL. */
export function shouldReuseLiveMcpHttp(opts: {
  hasHttpSession: boolean;
  sessionKey: string;
  overrideUrl?: string;
  settingsUrl: string;
  openedForSettingsUrl: string;
}): boolean {
  if (!opts.hasHttpSession || !opts.sessionKey) return false;
  const override = opts.overrideUrl?.trim();
  if (override) {
    const parsed = parseMcpHttpUrl(override);
    return typeof parsed === "string" && opts.sessionKey === `http\0${parsed}`;
  }
  return opts.openedForSettingsUrl === opts.settingsUrl.trim();
}

/** /models probe must not handshake a second session while chat_send is using this one. */
export function shouldReuseLiveMcpHttpForProbe(opts: { liveSessionKey?: string; probeUrl: string }): boolean {
  if (!opts.liveSessionKey) return false;
  const parsed = parseMcpHttpUrl(opts.probeUrl);
  return typeof parsed === "string" && opts.liveSessionKey === `http\0${parsed}`;
}

export type McpStayAction = "retry" | "fail" | "rethrow";

/** Agent = MCP: retry connect briefly, then fail. Do not switch to Late vLLM. */
export function decideMcpStay(opts: {
  message: string;
  attempt: number;
  maxAttempts?: number;
  aborted?: boolean;
}): McpStayAction {
  if (opts.aborted) return "rethrow";
  if (!isMcpConnectFailure(opts.message)) return "rethrow";
  const max = opts.maxAttempts ?? MCP_CONNECT_ATTEMPTS;
  if (opts.attempt < max) return "retry";
  return "fail";
}

export async function stayOnMcp<T>(opts: {
  run: () => Promise<T>;
  emit: (e: SseEvent) => void;
  signal: AbortSignal;
  reset?: () => void;
  delay?: (ms: number) => Promise<void>;
  attempts?: number;
  retryMs?: number;
}): Promise<T> {
  const attempts = opts.attempts ?? MCP_CONNECT_ATTEMPTS;
  const retryMs = opts.retryMs ?? MCP_RETRY_MS;
  const delay = opts.delay ?? sleep;
  let lastMsg = "MCP is not reachable.";
  for (let attempt = 1; attempt <= attempts; attempt++) {
    if (opts.signal.aborted) throw new Error("stopped");
    try {
      if (attempt > 1) {
        opts.reset?.();
        opts.emit({
          type: "heartbeat",
          message: `${MCP_UNREACHABLE_MESSAGE} — retry ${attempt - 1}…`,
        });
        await delay(retryMs * (attempt - 1));
      }
      return await opts.run();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      lastMsg = msg;
      const action = decideMcpStay({
        message: msg,
        attempt,
        maxAttempts: attempts,
        aborted: opts.signal.aborted,
      });
      if (action === "rethrow") throw err;
      if (action === "retry") continue;
      break;
    }
  }
  opts.emit({ type: "heartbeat", message: "MCP still unreachable. Paste the printed /mcp URL." });
  throw new Error(mcpStayUnreachableMessage(lastMsg));
}
