import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { cancelAll, decide, listPending } from "./approvals.js";
import { resolveSidecarChatBackend } from "./chat-backend.js";
import { runCursorChat } from "./cursor-loop.js";
import { auditEvent, authorizeLocal, corsAllowOrigin, isLoopbackHostHeader } from "./local-auth.js";
import {
  assertChatAllowed,
  chatTargetMeta,
  listLlamaCppModels,
  listLocalModels,
  listOllamaModels,
  probeChatTarget,
  runLlamaCppChat,
  runLocalChat,
  runOllamaChat,
} from "./openai-loop.js";
import {
  listNativeModels,
  nativeTargetMeta,
  probeNativeTarget,
  runAnthropicChat,
  runAzureChat,
  runGeminiChat,
  type NativeKind,
} from "./native-loop.js";
import { callMcpTool, probeMcp, listMcpAgents, mcpTargetMeta } from "./mcp-client.js";
import { grantMcpAllowedDir, parseGrantDirPath } from "./mcp-grant-dir.js";
import { runMcpChatOrFallback } from "./mcp-loop.js";
import { mcpSafeErrorMessage } from "./mcp-chat-format.js";
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
  let n = 0;
  const max = 2_000_000;
  for await (const c of req) {
    const buf = c as Buffer;
    n += buf.length;
    if (n > max) {
      req.destroy();
      throw new Error("request too large");
    }
    chunks.push(buf);
  }
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
      sendJson(res, req, 200, { ok: true });
      return;
    }
    if (!authorizeLocal(req, url)) {
      sendJson(res, req, 401, { error: "unauthorized" });
      return;
    }
    if (req.method === "GET" && url.pathname === "/models") {
      const ollama = await listOllamaModels();
      const llamacpp = await listLlamaCppModels();
      const [localTarget, ollamaTarget, llamacppTarget, anthropicTarget, geminiTarget, azureTarget, mcpTarget] =
        await Promise.all([
          chatTargetMeta("vllm"),
          chatTargetMeta("ollama"),
          chatTargetMeta("llamacpp"),
          nativeTargetMeta("anthropic"),
          nativeTargetMeta("gemini"),
          nativeTargetMeta("azure"),
          mcpTargetMeta(),
        ]);
      const mcp = await probeMcp();
      const mcpAgents = await listMcpAgents();
      sendJson(res, req, 200, {
        local: await listLocalModels(),
        cursor: ["composer-2.5", "auto"],
        ollama: ollama.models,
        ollamaOk: ollama.ok,
        ollamaMessage: ollama.message,
        llamacpp: llamacpp.models,
        llamacppOk: llamacpp.ok,
        llamacppMessage: llamacpp.message,
        anthropic: await listNativeModels("anthropic"),
        gemini: await listNativeModels("gemini"),
        azure: await listNativeModels("azure"),
        backends: ["local", "llamacpp", "ollama", "anthropic", "gemini", "azure", "cursor", "mcp"],
        targets: {
          local: localTarget,
          ollama: ollamaTarget,
          llamacpp: llamacppTarget,
          anthropic: anthropicTarget,
          gemini: geminiTarget,
          azure: azureTarget,
          mcp: mcpTarget,
        },
        mcp: { ...mcp, agents: mcpAgents },
      });
      return;
    }
    if (req.method === "POST" && url.pathname === "/probe") {
      const body = (await readJson(req)) as { kind?: string; base?: string };
      const raw = body.kind ?? "vllm";
      if (raw === "mcp") {
        const base = typeof body.base === "string" ? body.base.trim() : "";
        sendJson(res, req, 200, await probeMcp(true, base || undefined));
        return;
      }
      const native: NativeKind | null =
        raw === "anthropic" || raw === "gemini" || raw === "azure" ? raw : null;
      const kind =
        raw === "ollama" ? "ollama" : raw === "llamacpp" ? "llamacpp" : "vllm";
      if (
        raw !== "ollama" &&
        raw !== "llamacpp" &&
        raw !== "vllm" &&
        raw !== "local" &&
        !native
      ) {
        sendJson(res, req, 400, { error: "kind must be vllm, ollama, llamacpp, anthropic, gemini, azure, or mcp" });
        return;
      }
      const result = native
        ? await probeNativeTarget(native, typeof body.base === "string" ? body.base : undefined)
        : await probeChatTarget(kind, typeof body.base === "string" ? body.base : undefined);
      sendJson(res, req, 200, result);
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
    if (req.method === "POST" && url.pathname === "/mcp/grant-dir") {
      let raw: unknown;
      try {
        raw = await readJson(req);
      } catch (err) {
        sendJson(res, req, 400, { error: err instanceof Error ? err.message : "invalid JSON" });
        return;
      }
      const parsed = parseGrantDirPath(raw);
      if (!parsed.ok) {
        sendJson(res, req, 400, { error: parsed.error });
        return;
      }
      const result = await grantMcpAllowedDir(parsed.path, callMcpTool, (proposalId, path) => {
        auditEvent("mcp-grant-dir", { path, proposalId });
      });
      sendJson(res, req, result.status, result.body);
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
        backend?:
          | "local"
          | "cursor"
          | "ollama"
          | "llamacpp"
          | "llama.cpp"
          | "llama-cpp"
          | "llama_cpp"
          | "anthropic"
          | "gemini"
          | "azure"
          | "mcp";
        model?: string;
        conversationId?: string;
      };
      const conversationId = body.conversationId ?? crypto.randomUUID();
      const ac = new AbortController();
      runs.get(conversationId)?.abort();
      cancelAll("superseded by a new chat turn");
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
        const backend = resolveSidecarChatBackend(body.backend);
        if (backend === "cursor") {
          await assertChatAllowed("cursor");
        }
        // Agent=MCP is runMcpChatOrFallback (MCP only, never Late vLLM). Cursor SDK is only backend === "cursor".
        const text =
          backend === "cursor"
            ? await runCursorChat(chatOpts)
            : backend === "ollama"
              ? await runOllamaChat(chatOpts)
              : backend === "llamacpp"
                ? await runLlamaCppChat(chatOpts)
                : backend === "anthropic"
                  ? await runAnthropicChat(chatOpts)
                  : backend === "gemini"
                    ? await runGeminiChat(chatOpts)
                  : backend === "azure"
                    ? await runAzureChat(chatOpts)
                    : backend === "mcp"
                      ? await runMcpChatOrFallback(chatOpts)
                      : await runLocalChat(chatOpts);
        emit({ type: "done", message: text });
      } catch (err) {
        const raw = err instanceof Error ? err.message : String(err);
        emit({ type: "error", message: mcpSafeErrorMessage(raw) });
      } finally {
        runs.delete(conversationId);
        res.end();
      }
      return;
    }
    sendJson(res, req, 404, { error: "not found" });
  } catch (err) {
    try {
      if (!res.headersSent) {
        sendJson(res, req, 500, { error: err instanceof Error ? err.message : String(err) });
      } else {
        res.end();
      }
    } catch {
      /* client already gone — do not crash the sidecar */
    }
  }
});

server.listen(SIDECAR_PORT, "127.0.0.1", () => {
  console.log(`late-agent-sidecar http://127.0.0.1:${SIDECAR_PORT}`);
});
