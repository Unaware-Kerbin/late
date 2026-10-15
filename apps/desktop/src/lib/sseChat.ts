import type { SidecarEvent } from "./sidecar";

/** Bound incomplete SSE framing so a malformed/huge event cannot retain unbounded text. */
export const MAX_SSE_BUF_CHARS = 1_000_000;

/**
 * Cap an incomplete SSE buffer without slicing mid-JSON.
 * Prefer dropping whole completed frames from the front; if a single incomplete
 * event exceeds the cap, discard it entirely so a later `\n\n` cannot "complete"
 * a truncated JSON fragment into a corrupt Local AI tool/delta event.
 */
export function capIncompleteSseBuffer(buf: string, maxChars = MAX_SSE_BUF_CHARS): string {
  if (buf.length <= maxChars) return buf;
  // Over the cap: drop every completed frame and keep only the trailing
  // incomplete event (never slice mid-JSON). If that alone still exceeds
  // maxChars, discard it so a later `\n\n` cannot complete a corrupt prefix.
  const boundary = buf.lastIndexOf("\n\n");
  if (boundary >= 0) {
    const tail = buf.slice(boundary + 2);
    return tail.length <= maxChars ? tail : "";
  }
  return "";
}

/** Parse complete SSE frames from `buf + chunk`; return leftover incomplete text + events. */
export function ingestSseChatChunk(
  buf: string,
  chunk: string,
  maxChars = MAX_SSE_BUF_CHARS,
): { buf: string; events: SidecarEvent[] } {
  // Split complete frames first so Approve / tool / done events in the same
  // read as a huge delta are never dropped by the incomplete-buffer cap.
  const parts = (buf + chunk).split("\n\n");
  let next = capIncompleteSseBuffer(parts.pop() ?? "", maxChars);
  const events: SidecarEvent[] = [];
  for (const part of parts) {
    const dataLine = part.split("\n").find((l) => l.startsWith("data:"));
    if (!dataLine) continue;
    // Oversized complete frames (multi‑MiB deltas) — drop whole event, never
    // JSON.parse a truncated tail into a forged Local AI event.
    if (dataLine.length > maxChars) continue;
    try {
      events.push(JSON.parse(dataLine.slice(5).trim()) as SidecarEvent);
    } catch {
      /* ignore malformed */
    }
  }
  return { buf: next, events };
}
