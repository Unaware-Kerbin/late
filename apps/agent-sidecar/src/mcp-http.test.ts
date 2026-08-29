import assert from "node:assert/strict";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { describe, it } from "node:test";
import { parseMcpHttpUrl, parseSseJsonRpc } from "./mcp-format.ts";
import { formatMcpHttpStatusError, probeMcpHttpEndpoint } from "./mcp-http.ts";

function jsonRpc(res: ServerResponse, status: number, body: unknown) {
  res.writeHead(status, {
    "content-type": "application/json",
    "mcp-session-id": "late-test",
  });
  res.end(JSON.stringify(body));
}

function readBody(req: IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    req.on("data", (c: Buffer) => chunks.push(c));
    req.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
    req.on("error", reject);
  });
}

describe("mcp-http", () => {
  it("parses SSE JSON-RPC frames", () => {
    const msgs = parseSseJsonRpc(
      'event: message\ndata: {"jsonrpc":"2.0","id":1,"result":{"tools":[{"name":"list_agents"}]}}\n\n',
    );
    assert.equal(msgs.length, 1);
    assert.equal((msgs[0]?.result as { tools?: { name: string }[] })?.tools?.[0]?.name, "list_agents");
  });

  it("handshakes Streamable HTTP and lists tools", async () => {
    const server = createServer(async (req, res) => {
      if (req.method !== "POST") {
        res.writeHead(405);
        res.end();
        return;
      }
      const raw = await readBody(req);
      const msg = JSON.parse(raw) as { id?: number; method?: string };
      if (msg.method === "initialize") {
        jsonRpc(res, 200, {
          jsonrpc: "2.0",
          id: msg.id,
          result: { protocolVersion: "2025-03-26", capabilities: { tools: {} }, serverInfo: { name: "mock" } },
        });
        return;
      }
      if (msg.method === "notifications/initialized") {
        res.writeHead(202);
        res.end();
        return;
      }
      if (msg.method === "tools/list") {
        jsonRpc(res, 200, {
          jsonrpc: "2.0",
          id: msg.id,
          result: {
            tools: [{ name: "list_agents", description: "List specialists", inputSchema: { type: "object" } }],
          },
        });
        return;
      }
      jsonRpc(res, 200, { jsonrpc: "2.0", id: msg.id, error: { message: `unknown ${msg.method}` } });
    });
    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    const addr = server.address();
    assert.ok(addr && typeof addr === "object");
    const url = `http://127.0.0.1:${addr.port}/mcp`;
    try {
      const headers = {
        "Content-Type": "application/json",
        Accept: "application/json, text/event-stream",
        "MCP-Protocol-Version": "2025-03-26",
        Authorization: "Bearer late-test-token",
      };
      const init = await fetch(url, {
        method: "POST",
        headers,
        body: JSON.stringify({
          jsonrpc: "2.0",
          id: 1,
          method: "initialize",
          params: { protocolVersion: "2025-03-26", capabilities: { tools: {} }, clientInfo: { name: "late" } },
        }),
      });
      assert.equal(init.ok, true);
      await fetch(url, {
        method: "POST",
        headers,
        body: JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }),
      });
      const listed = await fetch(url, {
        method: "POST",
        headers,
        body: JSON.stringify({ jsonrpc: "2.0", id: 2, method: "tools/list", params: {} }),
      });
      const body = (await listed.json()) as { result?: { tools?: { name: string }[] } };
      assert.equal(body.result?.tools?.[0]?.name, "list_agents");
    } finally {
      await new Promise<void>((resolve, reject) => server.close((err) => (err ? reject(err) : resolve())));
    }
  });

  it("treats initialize/tools/list as MCP on even if GUI /health is 401 and GET /mcp is 405", async () => {
    const mcp = createServer(async (req, res) => {
      if (req.method === "GET") {
        res.writeHead(405, { "content-type": "application/json" });
        res.end(JSON.stringify({ jsonrpc: "2.0", error: { code: -32000, message: "Method not allowed." }, id: null }));
        return;
      }
      const raw = await readBody(req);
      const msg = JSON.parse(raw) as { id?: number; method?: string };
      if (msg.method === "initialize") {
        jsonRpc(res, 200, {
          jsonrpc: "2.0",
          id: msg.id,
          result: { protocolVersion: "2025-03-26", capabilities: { tools: {} }, serverInfo: { name: "mock" } },
        });
        return;
      }
      if (msg.method === "notifications/initialized") {
        res.writeHead(202);
        res.end();
        return;
      }
      if (msg.method === "tools/list") {
        jsonRpc(res, 200, {
          jsonrpc: "2.0",
          id: msg.id,
          result: {
            tools: [{ name: "list_agents", description: "List specialists", inputSchema: { type: "object" } }],
          },
        });
        return;
      }
      jsonRpc(res, 200, { jsonrpc: "2.0", id: msg.id, error: { message: `unknown ${msg.method}` } });
    });
    const gui = createServer((_req, res) => {
      res.writeHead(401, { "content-type": "application/json" });
      res.end(JSON.stringify({ error: "unauthorized" }));
    });
    await new Promise<void>((resolve) => mcp.listen(0, "127.0.0.1", resolve));
    await new Promise<void>((resolve) => gui.listen(0, "127.0.0.1", resolve));
    const mcpAddr = mcp.address();
    const guiAddr = gui.address();
    assert.ok(mcpAddr && typeof mcpAddr === "object");
    assert.ok(guiAddr && typeof guiAddr === "object");
    try {
      const guiHealth = await fetch(`http://127.0.0.1:${guiAddr.port}/health`);
      assert.equal(guiHealth.status, 401);
      const getMcp = await fetch(`http://127.0.0.1:${mcpAddr.port}/mcp`);
      assert.equal(getMcp.status, 405);

      const parsed = parseMcpHttpUrl(`http://localhost:${mcpAddr.port}/mcp`);
      assert.equal(parsed, `http://127.0.0.1:${mcpAddr.port}/mcp`);
      if (typeof parsed !== "string") return;
      const url = parsed;
      const headers = {
        "Content-Type": "application/json",
        Accept: "application/json, text/event-stream",
        "MCP-Protocol-Version": "2025-03-26",
      };
      const init = await fetch(url, {
        method: "POST",
        headers,
        body: JSON.stringify({
          jsonrpc: "2.0",
          id: 1,
          method: "initialize",
          params: {
            protocolVersion: "2025-03-26",
            capabilities: { tools: {} },
            clientInfo: { name: "late", version: "0.1.6" },
          },
        }),
      });
      assert.equal(init.ok, true, await init.clone().text());
      await fetch(url, {
        method: "POST",
        headers,
        body: JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }),
      });
      const listed = await fetch(url, {
        method: "POST",
        headers,
        body: JSON.stringify({ jsonrpc: "2.0", id: 2, method: "tools/list", params: {} }),
      });
      const body = (await listed.json()) as { result?: { tools?: { name: string }[] } };
      const names = (body.result?.tools ?? []).map((t) => t.name);
      assert.ok(names.includes("list_agents"), JSON.stringify(names));
      const n = names.length;
      const status = `MCP online · ${n} tool${n === 1 ? "" : "s"} at ${url}.`;
      assert.match(status, /MCP online/);
      assert.doesNotMatch(status, /not reachable|8787/);
    } finally {
      await Promise.all([
        new Promise<void>((resolve, reject) => mcp.close((err) => (err ? reject(err) : resolve()))),
        new Promise<void>((resolve, reject) => gui.close((err) => (err ? reject(err) : resolve()))),
      ]);
    }
  });

  it("connection refused to MCP HTTP fails fast and does not hang the helper", async () => {
    const started = Date.now();
    let failed = false;
    try {
      await fetch("http://127.0.0.1:9/mcp", {
        method: "POST",
        headers: { "content-type": "application/json", accept: "application/json, text/event-stream" },
        body: "{}",
        signal: AbortSignal.timeout(1500),
      });
    } catch {
      failed = true;
    }
    assert.equal(failed, true);
    assert.ok(Date.now() - started < 2000);
  });

  it("HTTP 400 surfaces the server error text, not MCP unreachable", async () => {
    assert.equal(
      formatMcpHttpStatusError(400, JSON.stringify({ jsonrpc: "2.0", error: { code: -32000, message: "Bad Request: Mcp-Session-Id header is required" }, id: null })),
      "MCP HTTP 400: Bad Request: Mcp-Session-Id header is required",
    );
    const server = createServer((_req, res) => {
      res.writeHead(400, { "content-type": "application/json" });
      res.end(
        JSON.stringify({
          jsonrpc: "2.0",
          error: { code: -32000, message: "Bad Request: Mcp-Session-Id header is required" },
          id: null,
        }),
      );
    });
    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    const addr = server.address();
    assert.ok(addr && typeof addr === "object");
    try {
      const probed = await probeMcpHttpEndpoint(`http://127.0.0.1:${addr.port}/mcp`);
      assert.equal(probed.ok, false);
      assert.match(probed.message, /MCP HTTP 400/);
      assert.match(probed.message, /Mcp-Session-Id header is required/);
      assert.doesNotMatch(probed.message, /unreachable|will not switch to the local vLLM helper/i);
    } finally {
      await new Promise<void>((resolve, reject) => server.close((err) => (err ? reject(err) : resolve())));
    }
  });
});
