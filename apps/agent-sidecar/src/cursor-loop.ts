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

  const toolNotes: string[] = [];
  let lastToolDoneAt = 0;
  const tools = cursorToolDefs(() => ctx);
  for (const def of Object.values(tools) as Record<string, unknown>[]) {
    for (const key of ["execute", "handler", "run"] as const) {
      const orig = def[key];
      if (typeof orig !== "function") continue;
      const bound = (orig as (a: Record<string, unknown>) => Promise<string>).bind(def);
      def[key] = async (args: Record<string, unknown>) => {
        const result = await bound(args);
        toolNotes.push(String(result).slice(0, 4000));
        lastToolDoneAt = Date.now();
        return result;
      };
    }
  }

  const linked = AbortSignal.any([opts.signal, AbortSignal.timeout(TURN_TIMEOUT_MS * 8)]);
  let agent: AgentHandle | undefined;

  function continuePrompt(): string {
    return [
      SYSTEM_PROMPT,
      "",
      "Live device output is already below.",
      "If that output answers the question, reply with the answer ONLY. Do not propose another show. Do not investigate related features.",
      "Only propose a command if a named fact is still missing, or if the operator asked you to implement a change and you now have the syntax.",
      "",
      `Original question:\n${lastUser?.content ?? ""}`,
      "",
      `Command output so far:\n${toolNotes.join("\n\n---\n")}`,
    ].join("\n");
  }

  async function oneShot(userPrompt: string): Promise<string> {
    agent = await Agent.create({
      apiKey,
      model: { id: opts.model ?? "composer-2.5" },
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
        customTools: tools,
        settingSources: [],
      },
    });
    const run = await agent.send(userPrompt);
    const iterator = run.stream()[Symbol.asyncIterator]();
    let text = "";
    let lastTextAt = Date.now();
    const toolsAtStart = toolNotes.length;
    const idleAfterToolsMs = 15_000;
    const idleAfterAnswerMs = 8_000;

    const closeStream = async () => {
      try {
        await iterator.return?.(undefined);
      } catch {
        /* ignore */
      }
    };

    const settle = async (): Promise<"none" | "done" | "stall"> => {
      const quietText = Date.now() - lastTextAt > idleAfterAnswerMs;
      const quietTool = Date.now() - lastToolDoneAt > idleAfterToolsMs;
      if (text.trim().length > 40 && quietText) {
        await closeStream();
        return "done";
      }
      if (toolNotes.length > toolsAtStart && quietTool && quietText && !text.trim()) {
        await closeStream();
        return "stall";
      }
      return "none";
    };

    while (true) {
      if (opts.signal.aborted || linked.aborted) throw new Error("stopped");
      const settled = await settle();
      if (settled === "done") return text;
      if (settled === "stall") {
        if (text.trim()) return text;
        throw new Error("Cursor went silent after a command");
      }
      const raced = await Promise.race([
        iterator.next().then((v) => ({ kind: "val" as const, v })),
        new Promise<{ kind: "tick" }>((resolve) => {
          setTimeout(() => resolve({ kind: "tick" }), 1000);
        }),
      ]);
      if (raced.kind === "tick") {
        const sinceTool = lastToolDoneAt ? Math.round((Date.now() - lastToolDoneAt) / 1000) : 0;
        const sinceText = Math.round((Date.now() - lastTextAt) / 1000);
        opts.emit({
          type: "heartbeat",
          message: text.trim()
            ? `Finishing the answer (${sinceText}s quiet)`
            : toolNotes.length > toolsAtStart
              ? `Waiting on Cursor after the command (${sinceTool}s)`
              : `Cursor is thinking (${sinceText}s)`,
        });
        continue;
      }
      if (raced.v.done) break;
      const e = raced.v.value as {
        type?: string;
        message?: { content?: { type: string; text?: string }[] };
      };
      if (e.type === "assistant" && e.message?.content) {
        for (const block of e.message.content) {
          if (block.type === "text" && block.text?.trim()) {
            text += block.text;
            lastTextAt = Date.now();
            opts.emit({ type: "delta", text: block.text });
          }
        }
      }
    }
    const result = await Promise.race([
      run.wait(),
      new Promise<{ result?: string }>((resolve) => setTimeout(() => resolve({}), 8000)),
    ]);
    return text || result.result || "";
  }

  try {
    try {
      const text = await oneShot(prompt);
      if (text.trim() || !toolNotes.length) return text;
    } catch (err) {
      if (!toolNotes.length || opts.signal.aborted) throw err;
      opts.emit({
        type: "delta",
        text: `\n(Cursor went quiet after the command. Not starting another think loop.)\n`,
      });
      return "";
    }
    opts.emit({ type: "delta", text: "\n(Answering from device output.)\n" });
    return await oneShot(continuePrompt());
  } finally {
    try {
      await agent?.[Symbol.asyncDispose]?.();
    } catch {
      /* ignore */
    }
    await rm(scratch, { recursive: true, force: true });
  }
}
