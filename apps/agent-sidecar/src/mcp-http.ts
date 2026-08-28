import { fetchStayOnBox } from "./chat-target.js";
import { GUI_NOT_MCP, mcpHttpProbeUrls, parseMcpHttpUrl, parseSseJsonRpc, type JsonRpcMsg } from "./mcp-format.js";
import { mcpTransportError, unreachableMcp } from "./mcp-transport.js";

export { mcpTransportError, unreachableMcp } from "./mcp-transport.js";

export type JsonRpc = JsonRpcMsg;

export type McpHttpSession = {
  url: string;
  endpoint: string;
  sessionId: string;
  nextId: number;
};

const PROTOCOL = "2025-03-26";

function describeHttpError(err: unknown, url: string): Error {
  return mcpTransportError(err, url);
}

function sseEndpointUrl(text: string, base: string): string | null {
  let event = "";
  let data: string[] = [];
  const flush = (): string | null => {
    if (event === "endpoint" && data.length) {
      const raw = data.join("").trim();
      if (!raw) return null;
      try {
        return new URL(raw, base).toString();
      } catch {
        return null;
      }
    }
    event = "";
    data = [];
    return null;
  };
  for (const line of text.split(/\r?\n/)) {
    if (line.startsWith("event:")) {
      event = line.slice(6).trim();
    } else if (line.startsWith("data:")) {
      data.push(line.slice(5).trim());
    } else if (line === "") {
      const found = flush();
      if (found) return found;
    }
  }
  return flush();
}

function resultForId(messages: JsonRpc[], id: number): unknown {
  const match = messages.find((m) => Number(m.id) === id);
  if (!match) throw new Error("MCP HTTP response had no matching id");
  if (match.error) {
    throw new Error(match.error.message || `MCP error ${match.error.code ?? ""}`.trim());
  }
  return match.result;
}

function mcpHeaders(sessionId: string): Record<string, string> {
  const h: Record<string, string> = {
    "Content-Type": "application/json",
    Accept: "application/json, text/event-stream",
    "MCP-Protocol-Version": PROTOCOL,
  };
  if (sessionId) h["Mcp-Session-Id"] = sessionId;
  return h;
}

function looksLikeHtml(ctype: string, text: string): boolean {
  if (ctype.includes("text/html")) return true;
  const t = text.trimStart().slice(0, 64).toLowerCase();
  return t.startsWith("<!doctype html") || t.startsWith("<html");
}

export function htmlPageNotMcp(_url: string): Error {
  return new Error(GUI_NOT_MCP);
}

async function readRpcResponse(r: Response, id: number, url: string): Promise<{ result: unknown; sessionId: string }> {
  const sessionId = r.headers.get("mcp-session-id") ?? "";
  if (!r.ok && r.status >= 400 && r.status !== 202) {
    const text = await r.text().catch(() => "");
    if (r.status === 404 || r.status === 502 || r.status === 503) throw unreachableMcp(url);
    if (looksLikeHtml((r.headers.get("content-type") ?? "").toLowerCase(), text)) {
      throw htmlPageNotMcp(url);
    }
    throw new Error(`MCP HTTP ${r.status}${text ? `: ${text.slice(0, 200)}` : ""}`);
  }
  const ctype = (r.headers.get("content-type") ?? "").toLowerCase();
  if (r.status === 202) return { result: undefined, sessionId };
  if (ctype.includes("text/event-stream")) {
    const text = await r.text();
    return { result: resultForId(parseSseJsonRpc(text), id), sessionId };
  }
  const text = await r.text();
  if (looksLikeHtml(ctype, text)) throw htmlPageNotMcp(url);
  if (!text.trim()) return { result: undefined, sessionId };
  let msg: JsonRpc;
  try {
    msg = JSON.parse(text) as JsonRpc;
  } catch {
    throw new Error(`MCP HTTP returned non-JSON (${r.status}): ${text.slice(0, 200)}`);
  }
  if (msg.error) {
    throw new Error(msg.error.message || `MCP error ${msg.error.code ?? ""}`.trim());
  }
  return { result: msg.result, sessionId };
}

export async function mcpHttpRpc(
  sess: McpHttpSession,
  method: string,
  params?: unknown,
  timeoutMs = 8000,
): Promise<unknown> {
  const id = sess.nextId++;
  const payload = { jsonrpc: "2.0", id, method, params: params ?? {} };
  try {
    const r = await fetchStayOnBox(sess.endpoint, {
      method: "POST",
      headers: mcpHeaders(sess.sessionId),
      body: JSON.stringify(payload),
      signal: AbortSignal.timeout(timeoutMs),
    });
    const { result, sessionId } = await readRpcResponse(r, id, sess.url);
    if (sessionId) sess.sessionId = sessionId;
    return result;
  } catch (err) {
    throw describeHttpError(err, sess.url);
  }
}

export async function mcpHttpNotify(sess: McpHttpSession, method: string, params?: unknown): Promise<void> {
  try {
    await fetchStayOnBox(sess.endpoint, {
      method: "POST",
      headers: mcpHeaders(sess.sessionId),
      body: JSON.stringify({ jsonrpc: "2.0", method, params: params ?? {} }),
      signal: AbortSignal.timeout(5000),
    });
  } catch (err) {
    throw describeHttpError(err, sess.url);
  }
}

export type McpHttpProbe = {
  ok: boolean;
  tools: string[];
  message: string;
  url: string;
};

/**
 * MCP is on iff initialize + tools/list succeed on the Streamable HTTP URL.
 * Does not touch GUI :8787. Connection failures say unreachable; 401/403/405 keep their text.
 */
export async function probeMcpHttpEndpoint(rawUrl: string): Promise<McpHttpProbe> {
  const parsed = parseMcpHttpUrl(rawUrl);
  const shown = typeof parsed === "string" ? parsed : rawUrl.trim();
  try {
    const sess = await openMcpHttpSession(rawUrl);
    const result = (await mcpHttpRpc(sess, "tools/list")) as { tools?: { name?: string }[] };
    const tools = Array.isArray(result?.tools) ? result.tools : [];
    const names = tools.map((t) => String(t?.name ?? "").trim()).filter(Boolean);
    return {
      ok: true,
      tools: names,
      url: sess.url,
      message: `MCP online · ${names.length} tool${names.length === 1 ? "" : "s"} at ${sess.url}.`,
    };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    return { ok: false, tools: [], url: shown, message };
  }
}

async function trySseEndpoint(url: string): Promise<string | null> {
  try {
    const r = await fetchStayOnBox(url, {
      method: "GET",
      headers: { Accept: "text/event-stream" },
      signal: AbortSignal.timeout(4000),
    });
    if (!r.ok) return null;
    const ctype = (r.headers.get("content-type") ?? "").toLowerCase();
    if (!ctype.includes("text/event-stream")) return null;
    const text = await r.text();
    return sseEndpointUrl(text, url);
  } catch {
    return null;
  }
}

async function handshake(sess: McpHttpSession): Promise<void> {
  await mcpHttpRpc(sess, "initialize", {
    protocolVersion: PROTOCOL,
    capabilities: { tools: {} },
    clientInfo: { name: "late", version: "0.1.6" },
  });
  await mcpHttpNotify(sess, "notifications/initialized");
}

export async function openMcpHttpSession(rawUrl: string): Promise<McpHttpSession> {
  const candidates = mcpHttpProbeUrls(rawUrl);
  if (!Array.isArray(candidates)) {
    throw new Error(candidates.error);
  }
  let last: unknown;
  for (const parsed of candidates) {
    const sess: McpHttpSession = { url: parsed, endpoint: parsed, sessionId: "", nextId: 1 };
    try {
      await handshake(sess);
      return sess;
    } catch (first) {
      last = first;
      const alt = await trySseEndpoint(parsed);
      if (alt) {
        sess.endpoint = alt;
        sess.nextId = 1;
        try {
          await handshake(sess);
          return sess;
        } catch (second) {
          last = second;
        }
      }
    }
  }
  throw last instanceof Error ? last : new Error(String(last));
}
