/** Cap coalesced PTY text waiting for xterm.write so a burst cannot retain the full session. */
export const MAX_WRITE_PENDING_CHARS = 512_000;

export function appendWritePending(
  pending: string,
  text: string,
  maxChars = MAX_WRITE_PENDING_CHARS,
): string {
  if (!text) return pending;
  const next = pending ? pending + text : text;
  if (next.length <= maxChars) return next;
  return next.slice(-maxChars);
}
