import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { cancelAll, decide, listPending } from "./approvals.js";
import { runCursorChat } from "./cursor-loop.js";
import { daemon } from "./daemon.js";
import { auditEvent, authorizeLocal, corsAllowOrigin, isLoopbackHostHeader } from "./local-auth.js";
import { listLocalModels, listOllamaModels, runLocalChat, runOllamaChat } from "./openai-loop.js";
import { SIDECAR_PORT, type ApprovalDecision, type ChatMessage, type SseEvent } from "./types.js";

const runs = new Map<string, AbortController>();

function cors(res: ServerResponse, req: IncomingMessage) {
  const origin = typeof req.headers.origin === "string" ? req.headers.origin : "";
  const allow = corsAllowOrigin(origin);
  if (allow) {
    res.setHeader("Access-Control-Allow-Origin", allow);
    res.setHeader("Vary", "Origin");
    res.setHeader("Access-Control-Allow-Headers", "Content-Type, X-Late-Token, Authorization");
    res.setHeader("Access-Control-Allow-Methods", "GET,POST,OPTIONS");
  }
}

async function readJson(req: IncomingMessage): Promise<unknown> {
  const chunks: Buffer[] = [];
  for await (const c of req) chunks.push(c as Buffer);
  const raw = Buffer.concat(chunks).toString("utf8");
  if (!raw.trim()) return {};
  return JSON.parse(raw);
}

function sendJson(res: ServerResponse, req: IncomingMessage, status: number, body: unknown) {
  cors(res, req);
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(JSON.stringify(body));
}

function writeSse(res: ServerResponse, event: SseEvent) {
  res.write(`event: ${event.type}\ndata: ${JSON.stringify(event)}\n\n`);
}

const server = createServer(async (req, res) => {
  cors(res, req);
  if (req.method === "OPTIONS") {
    res.writeHead(204);
    res.end();
    return;
  }
  const url = new URL(req.url ?? "/", `http://127.0.0.1:${SIDECAR_PORT}`);
  try {
    if (!isLoopbackHostHeader(typeof req.headers.host === "string" ? req.headers.host : undefined)) {
      sendJson(res, req, 403, { error: "host not allowed" });
      return;
    }
    if (req.method === "GET" && url.pathname === "/health") {
      sendJson(res, req, 200, { ok: true, daemon: await daemon.health() });
      return;
    }
    if (!authorizeLocal(req, url)) {
      sendJson(res, req, 401, { error: "unauthorized" });
      return;
    }
    if (req.method === "GET" && url.pathname === "/models") {
      const ollama = await listOllamaModels();
      sendJson(res, req, 200, {
        local: await listLocalModels(),
        cursor: ["composer-2.5", "auto"],
        ollama: ollama.models,
        ollamaOk: ollama.ok,
        ollamaMessage: ollama.message,
        backends: ["local", "cursor", "ollama"],
      });
      return;
    }
    if (req.method === "GET" && url.pathname === "/pending") {
      sendJson(res, req, 200, { pending: listPending() });
      return;
    }
    if (req.method === "POST" && url.pathname === "/approve") {
      const body = (await readJson(req)) as ApprovalDecision;
      if (!body?.proposalId) {
        sendJson(res, req, 400, { error: "proposalId required" });
        return;
      }
      const ok = decide({
        proposalId: body.proposalId,
        allow: Boolean(body.allow),
        alwaysAllow: Boolean(body.alwaysAllow),
        answer: body.answer,
      });
      auditEvent("approve", { ok, allow: Boolean(body.allow), alwaysAllow: Boolean(body.alwaysAllow) });
      sendJson(res, req, ok ? 200 : 404, { ok });
      return;
    }
    if (req.method === "POST" && url.pathname === "/stop") {
      const body = (await readJson(req)) as { conversationId?: string };
      if (body.conversationId) {
        runs.get(body.conversationId)?.abort();
        runs.delete(body.conversationId);
      } else {
        for (const c of runs.values()) c.abort();
        runs.clear();
      }
      cancelAll("stopped");
      sendJson(res, req, 200, { ok: true });
      return;
    }
    if (req.method === "POST" && url.pathname === "/chat") {
      const body = (await readJson(req)) as {
        messages?: ChatMessage[];
        backend?: "local" | "cursor" | "ollama";
        model?: string;
        conversationId?: string;
      };
      const conversationId = body.conversationId ?? crypto.randomUUID();
      const ac = new AbortController();
      runs.get(conversationId)?.abort();
      runs.set(conversationId, ac);

      const origin = typeof req.headers.origin === "string" ? req.headers.origin : "";
      const sseHeaders: Record<string, string> = {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-cache",
        Connection: "keep-alive",
      };
      const allow = corsAllowOrigin(origin);
      if (allow) {
        sseHeaders["Access-Control-Allow-Origin"] = allow;
        sseHeaders.Vary = "Origin";
      }
      res.writeHead(200, sseHeaders);
      const emit = (e: SseEvent) => writeSse(res, e);
      try {
        const chatOpts = {
          messages: body.messages ?? [],
          model: body.model,
          conversationId,
          emit,
          signal: ac.signal,
        };
        const text =
          body.backend === "cursor"
            ? await runCursorChat(chatOpts)
            : body.backend === "ollama"
              ? await runOllamaChat(chatOpts)
              : await runLocalChat(chatOpts);
        emit({ type: "done", message: text });
      } catch (err) {
        emit({ type: "error", message: err instanceof Error ? err.message : String(err) });
      } finally {
        runs.delete(conversationId);
        res.end();
      }
      return;
    }
    sendJson(res, req, 404, { error: "not found" });
  } catch (err) {
    sendJson(res, req, 500, { error: err instanceof Error ? err.message : String(err) });
  }
});

server.listen(SIDECAR_PORT, "127.0.0.1", () => {
  console.log(`late-agent-sidecar http://127.0.0.1:${SIDECAR_PORT}`);
});
