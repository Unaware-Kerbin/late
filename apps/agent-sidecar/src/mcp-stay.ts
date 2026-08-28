import type { SseEvent } from "./types.js";

export const MCP_CONNECT_ATTEMPTS = 3;
export const MCP_RETRY_MS = 400;

/** Short heartbeat prefix. The thrown error adds the no-vLLM-fallback note. */
export const MCP_UNREACHABLE_MESSAGE = "MCP unreachable";

export function mcpStayUnreachableMessage(_detail = ""): string {
  return `${MCP_UNREACHABLE_MESSAGE}. Chat stays on MCP — Late will not switch to the local vLLM helper (often :8000).`;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    const t = setTimeout(resolve, ms);
    t.unref?.();
  });
}

/** Connection failures to the MCP program itself (not debate timeouts, not Late vLLM :8000). */
export function isMcpConnectFailure(message: string): boolean {
  if (/vLLM is not reachable|127\.0\.0\.1:8000/i.test(message)) return false;
  return /MCP is not reachable|ECONNREFUSED|ECONNRESET|ETIMEDOUT|UND_ERR_CONNECT_TIMEOUT|UND_ERR_SOCKET|fetch failed|MCP HTTP 404|MCP HTTP 502|MCP HTTP 503|MCP server exited|MCP request timed out|MCP initialize timed out/i.test(
    message,
  );
}

export function isMcpUnavailable(message: string): boolean {
  return isMcpConnectFailure(message);
}

export type McpStayAction = "retry" | "fail" | "rethrow";

/** Agent = MCP: retry connect briefly, then stay on MCP. Never fall back to Late vLLM. */
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
  opts.emit({ type: "heartbeat", message: "MCP still unreachable — staying on MCP." });
  throw new Error(mcpStayUnreachableMessage(lastMsg));
}
