import { loadProviderKeys } from "./provider-keys.js";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { cursorToolDefs, liveSessionContext, type ToolCtx } from "./tools.js";
import { SYSTEM_PROMPT, TURN_TIMEOUT_MS, type ChatMessage, type SseEvent } from "./types.js";

type AgentHandle = {
  send: (prompt: string) => Promise<{
    stream: () => AsyncIterable<Record<string, unknown>>;
    wait: () => Promise<{ result?: string }>;
  }>;
  [Symbol.asyncDispose]?: () => Promise<void>;
};

export async function runCursorChat(opts: {
  messages: ChatMessage[];
  model?: string;
  conversationId: string;
  emit: (e: SseEvent) => void;
  signal: AbortSignal;
}): Promise<string> {
  const stored = await loadProviderKeys();
  const apiKey = process.env.CURSOR_API_KEY || process.env.CURSOR_API_KEY || stored.cursor;
  if (!apiKey) {
    throw new Error(
      "No Cursor API key. Save one in Settings → API keys, or export CURSOR_API_KEY for this process.",
    );
  }

  let Agent: { create: (o: Record<string, unknown>) => Promise<AgentHandle> };
  try {
    const mod = (await import("@cursor/sdk")) as { Agent: typeof Agent };
    Agent = mod.Agent;
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    throw new Error(
      `Cursor SDK (@cursor/sdk) failed to import: ${message}. Install the optional dependency and retry, or switch the chat backend to local.`,
    );
  }

  const scratch = await mkdtemp(join(tmpdir(), "late-scratch-"));
  const ctx: ToolCtx = {
    conversationId: opts.conversationId,
    emit: opts.emit,
    signal: opts.signal,
  };

  const sessionBlock = await liveSessionContext();
  const history = opts.messages.filter((m) => m.role !== "system");
  const lastUser = [...history].reverse().find((m) => m.role === "user");
  const prompt = [
    SYSTEM_PROMPT,
    "",
    sessionBlock,
    "",
    ...history.map((m) => {
      const body = m.content ?? "";
      if (m === lastUser) {
        return `USER: ${body}\n\n---\n${sessionBlock}`;
      }
      return `${m.role.toUpperCase()}: ${body}`;
    }),
  ].join("\n");

  const linked = AbortSignal.any([opts.signal, AbortSignal.timeout(TURN_TIMEOUT_MS * 8)]);
  let agent: AgentHandle | undefined;
  try {
    agent = await Agent.create({
      apiKey,
      model: { id: opts.model ?? "composer-2.5" },
      // [] disables MCP entirely, including custom Late tools. "mcp" keeps
      // only our in-process tools (no shell/read/edit).
      tools: ["mcp"],
      disallowedTools: [
        "shell",
        "read",
        "edit",
        "grep",
        "glob",
        "ls",
        "task",
        "webSearch",
        "webFetch",
        "delete",
        "generateImage",
      ],
      mcpServers: {},
      local: {
        cwd: scratch,
        customTools: cursorToolDefs(() => ctx),
        settingSources: [],
      },
    });

    const run = await agent.send(prompt);
    let text = "";
    for await (const event of run.stream()) {
      if (linked.aborted) break;
      const e = event as {
        type?: string;
        message?: { content?: { type: string; text?: string }[] };
      };
      if (e.type === "assistant" && e.message?.content) {
        for (const block of e.message.content) {
          if (block.type === "text" && block.text) {
            text += block.text;
            opts.emit({ type: "delta", text: block.text });
          }
        }
      }
    }
    if (linked.aborted) throw new Error("stopped");
    const result = await run.wait();
    return text || result.result || "";
  } finally {
    try {
      await agent?.[Symbol.asyncDispose]?.();
    } catch {
      /* ignore */
    }
    await rm(scratch, { recursive: true, force: true });
  }
}
