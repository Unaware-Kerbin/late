import { loadProviderKeys } from "./provider-keys.js";
import { randomUUID } from "node:crypto";
import { executeTool, liveSessionContext, OPENAI_TOOLS, type ToolCtx } from "./tools.js";
import {
  MAX_ROUNDS,
  SYSTEM_PROMPT,
  TURN_TIMEOUT_MS,
  VLLM_BASE,
  type ChatMessage,
  type SseEvent,
  type ToolCall,
} from "./types.js";

export async function runLocalChat(opts: {
  messages: ChatMessage[];
  model?: string;
  conversationId: string;
  emit: (e: SseEvent) => void;
  signal: AbortSignal;
}): Promise<string> {
  const model = opts.model ?? (await defaultModel());
  const live = await liveSessionContext();
  const messages: ChatMessage[] = [
    { role: "system", content: SYSTEM_PROMPT },
    { role: "system", content: live },
    ...opts.messages.filter((m) => m.role !== "system"),
  ];

  let assistantText = "";
  for (let round = 1; round <= MAX_ROUNDS; round++) {
    if (opts.signal.aborted) throw new Error("stopped");
    opts.emit({ type: "round", n: round, max: MAX_ROUNDS });
    messages[1] = { role: "system", content: await liveSessionContext() };

    const completion = await chatCompletions(model, messages, opts.signal);
    const choice = completion.choices?.[0];
    if (!choice) throw new Error("vLLM returned no choices");

    const msg = choice.message as ChatMessage;
    const toolCalls: ToolCall[] = msg.tool_calls ?? [];
    const content = msg.content ?? "";

    if (content) {
      opts.emit({ type: "delta", text: content });
      assistantText += content;
    }

    if (!toolCalls.length) return assistantText;

    messages.push({ role: "assistant", content: content || null, tool_calls: toolCalls });

    const ctx: ToolCtx = {
      conversationId: opts.conversationId,
      emit: opts.emit,
      signal: opts.signal,
    };
    for (const call of toolCalls) {
      const result = await executeTool(call.function.name, call.function.arguments ?? "{}", ctx);
      messages.push({
        role: "tool",
        tool_call_id: call.id || randomUUID(),
        name: call.function.name,
        content: result,
      });
    }
  }
  throw new Error(`agent hit the ${MAX_ROUNDS}-round cap`);
}

async function defaultModel(): Promise<string> {
  const models = await listLocalModels();
  return models[0] ?? "local";
}

async function chatCompletions(model: string, messages: ChatMessage[], signal: AbortSignal) {
  const linked = AbortSignal.any([signal, AbortSignal.timeout(TURN_TIMEOUT_MS)]);
  const r = await fetch(`${VLLM_BASE}/chat/completions`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      ...(await bearerFor(VLLM_BASE)),
    },
    signal: linked,
    body: JSON.stringify({
      model,
      messages,
      tools: OPENAI_TOOLS,
      tool_choice: "auto",
      temperature: 0.2,
      stream: false,
    }),
  });
  const text = await r.text();
  let json: { choices?: { message: ChatMessage }[]; error?: unknown };
  try {
    json = JSON.parse(text) as typeof json;
  } catch {
    throw new Error(`vLLM returned non-JSON (${r.status}): ${text.slice(0, 240)}`);
  }
  if (!r.ok) throw new Error(`vLLM ${r.status}: ${JSON.stringify(json.error ?? json)}`);
  return json;
}

async function bearerFor(base: string): Promise<Record<string, string>> {
  const local =
    base.includes("127.0.0.1") || base.includes("localhost") || base.includes("::1");
  if (local) return {};
  const keys = await loadProviderKeys();
  const key = keys.openai || keys.openrouter || keys.groq || keys.custom;
  return key ? { Authorization: `Bearer ${key}` } : {};
}

export async function listLocalModels(): Promise<string[]> {
  try {
    const r = await fetch(`${VLLM_BASE}/models`, { signal: AbortSignal.timeout(2000) });
    if (!r.ok) return [];
    const body = (await r.json()) as { data?: { id: string }[] };
    return (body.data ?? []).map((m) => m.id).filter((id) => !id.includes(":cloud"));
  } catch {
    return [];
  }
}
