import assert from "node:assert/strict";
import test from "node:test";
import { mcpTransportError, unreachableMcp } from "./mcp-transport.ts";

test("connect refused is unreachable at the MCP URL, not Late vLLM :8000", () => {
  const url = "http://127.0.0.1:8790/mcp";
  const refused = Object.assign(new Error("fetch failed"), { cause: { code: "ECONNREFUSED" } });
  const msg = mcpTransportError(refused, url).message;
  assert.match(msg, /MCP is not reachable at http:\/\/127\.0\.0\.1:8790\/mcp/);
  assert.doesNotMatch(msg, /8000/);
  assert.match(unreachableMcp(url).message, /8790\/mcp/);
});

test("in-flight abort is a timeout, not MCP down", () => {
  const url = "http://127.0.0.1:8790/mcp";
  const timed = Object.assign(new Error("The operation was aborted due to timeout"), {
    name: "TimeoutError",
  });
  const msg = mcpTransportError(timed, url).message;
  assert.match(msg, /MCP request timed out at http:\/\/127\.0\.0\.1:8790\/mcp/);
  assert.doesNotMatch(msg, /not reachable/);
});
