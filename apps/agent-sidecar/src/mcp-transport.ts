/** Classify MCP HTTP fetch failures. Connect errors are “MCP down”; in-flight abort is a timeout. */

export function unreachableMcp(url: string): Error {
  return new Error(
    `MCP is not reachable at ${url}. Start that program on that machine; Late will not start it.`,
  );
}

export function mcpTransportError(err: unknown, url: string): Error {
  const cause =
    err instanceof Error ? (err as Error & { cause?: { code?: string } }).cause : undefined;
  const code = cause?.code ?? "";
  const msg = err instanceof Error ? err.message : String(err);
  const name = err instanceof Error ? err.name : "";
  if (
    code === "ECONNREFUSED" ||
    code === "ECONNRESET" ||
    code === "ETIMEDOUT" ||
    code === "UND_ERR_CONNECT_TIMEOUT" ||
    code === "UND_ERR_SOCKET" ||
    /ECONNREFUSED|ECONNRESET|UND_ERR_CONNECT_TIMEOUT|UND_ERR_SOCKET/i.test(msg)
  ) {
    return unreachableMcp(url);
  }
  if (name === "TimeoutError" || name === "AbortError" || /aborted due to timeout|AbortError/i.test(msg)) {
    return new Error(`MCP request timed out at ${url}`);
  }
  if (/fetch failed/i.test(msg)) return unreachableMcp(url);
  return err instanceof Error ? err : new Error(String(err));
}
