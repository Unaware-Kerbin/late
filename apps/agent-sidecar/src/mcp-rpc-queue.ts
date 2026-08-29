/**
 * One in-flight JSON-RPC on the live MCP session.
 * Overlapping chat_get + add_allowed_dir (folder drop) can reset Streamable HTTP
 * and look like the orchestrator died.
 */
export function createRpcQueue(): <T>(fn: () => Promise<T>) => Promise<T> {
  let tail: Promise<void> = Promise.resolve();
  return function enqueue<T>(fn: () => Promise<T>): Promise<T> {
    const run = tail.then(fn, fn);
    tail = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  };
}
