import { randomUUID } from "node:crypto";
import { daemon } from "./daemon.js";
import { loadProviderKeys } from "./provider-keys.js";
import { executeTool, liveSessionContext, OPENAI_TOOLS, type ToolCtx } from "./tools.js";
import {
  MAX_ROUNDS,
  OLLAMA_BASE,
  SYSTEM_PROMPT,
  TURN_TIMEOUT_MS,
  VLLM_BASE,
  type ChatMessage,
  type SseEvent,
  type ToolCall,
} from "./types.js";

type CompatKind = "vllm" | "ollama";

export type OllamaProbe = {
  models: string[];
  ok: boolean;
  message: string;
};

let settingsCache: { at: number; value: Record<string, unknown> } | null = null;

async function loadAppSettings(): Promise<Record<string, unknown>> {
  if (settingsCache && Date.now() - settingsCache.at < 3000) return settingsCache.value;
  try {
    const value = await daemon.call<Record<string, unknown>>("settings.get");
    if (value && typeof value === "object") {
      settingsCache = { at: Date.now(), value };
      return value;
    }
  } catch {
    /* env and defaults still work if the daemon is down */
  }
  return settingsCache?.value ?? {};
}

function settingStr(s: Record<string, unknown>, ...keys: string[]): string | undefined {
  for (const k of keys) {
    const v = s[k];
    if (typeof v === "string" && v.trim()) return v.trim();
  }
  return undefined;
}

function trimSlash(url: string): string {
  return url.replace(/\/+$/, "");
}

function openaiV1(base: string): string {
  const trimmed = trimSlash(base);
  return /\/v1$/i.test(trimmed) ? trimmed : `${trimmed}/v1`;
}

function openaiRoot(base: string): string {
  return trimSlash(base).replace(/\/v1$/i, "");
}

async function resolveBase(kind: CompatKind): Promise<string> {
  if (kind === "ollama") {
    if (process.env.LATE_OLLAMA_BASE?.trim()) return openaiV1(process.env.LATE_OLLAMA_BASE);
    const s = await loadAppSettings();
    return openaiV1(settingStr(s, "ollamaBaseUrl", "ollama_base_url") ?? OLLAMA_BASE);
  }
  if (process.env.LATE_VLLM_BASE?.trim()) return openaiV1(process.env.LATE_VLLM_BASE);
  const s = await loadAppSettings();
  return openaiV1(settingStr(s, "vllmBaseUrl", "vllm_base_url") ?? VLLM_BASE);
}

function ollamaHelp(base: string, kind: "offline" | "empty"): string {
  const root = openaiRoot(base);
  if (kind === "empty") {
    return `Ollama is running at ${root} but has no models. Run \`ollama pull <model>\` (for example \`ollama pull llama3.2\`). Late does not pull models for you.`;
  }
  return `Ollama is not running at ${root}. Install from https://ollama.com, start it, then run \`ollama pull <model>\`. Late does not install Ollama or download models.`;
}

export async function runLocalChat(opts: {
  messages: ChatMessage[];
  model?: string;
  conversationId: string;
  emit: (e: SseEvent) => void;
  signal: AbortSignal;
}): Promise<string> {
  return runCompatChat({ ...opts, kind: "vllm" });
}

export async function runOllamaChat(opts: {
  messages: ChatMessage[];
  model?: string;
  conversationId: string;
  emit: (e: SseEvent) => void;
  signal: AbortSignal;
}): Promise<string> {
  return runCompatChat({ ...opts, kind: "ollama" });
}

async function runCompatChat(opts: {
  kind: CompatKind;
  messages: ChatMessage[];
  model?: string;
  conversationId: string;
  emit: (e: SseEvent) => void;
  signal: AbortSignal;
}): Promise<string> {
  const label = opts.kind === "ollama" ? "Ollama" : "vLLM";
  const base = await resolveBase(opts.kind);
  if (opts.kind === "ollama") {
    const probe = await probeOllama(base);
    if (!probe.ok) throw new Error(probe.message);
  }
  const model = opts.model || (await defaultModel(opts.kind, base));
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

    const completion = await chatCompletions(label, base, model, messages, opts.signal);
    const choice = completion.choices?.[0];
    if (!choice) throw new Error(`${label} returned no choices`);

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

async function defaultModel(kind: CompatKind, base: string): Promise<string> {
  if (kind === "ollama") {
    const probe = await probeOllama(base);
    const s = await loadAppSettings();
    const preferred = settingStr(s, "ollamaModel", "ollama_model");
    if (preferred && probe.models.includes(preferred)) return preferred;
    return probe.models[0] ?? "llama3.2";
  }
  const models = await listModelsAt(base);
  const s = await loadAppSettings();
  const preferred = settingStr(s, "vllmModel", "vllm_model");
  if (preferred && models.includes(preferred)) return preferred;
  return models[0] ?? "local";
}

async function chatCompletions(
  label: string,
  base: string,
  model: string,
  messages: ChatMessage[],
  signal: AbortSignal,
) {
  const linked = AbortSignal.any([signal, AbortSignal.timeout(TURN_TIMEOUT_MS)]);
  const r = await fetch(`${base}/chat/completions`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      ...(await bearerFor(base)),
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
    throw new Error(`${label} returned non-JSON (${r.status}): ${text.slice(0, 240)}`);
  }
  if (!r.ok) throw new Error(`${label} ${r.status}: ${JSON.stringify(json.error ?? json)}`);
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

async function listModelsAt(base: string): Promise<string[]> {
  try {
    const r = await fetch(`${base}/models`, { signal: AbortSignal.timeout(2000) });
    if (!r.ok) return [];
    const body = (await r.json()) as { data?: { id: string }[] };
    return (body.data ?? []).map((m) => m.id).filter((id) => !id.includes(":cloud"));
  } catch {
    return [];
  }
}

export async function listLocalModels(): Promise<string[]> {
  return listModelsAt(await resolveBase("vllm"));
}

async function probeOllama(base: string): Promise<OllamaProbe> {
  const root = openaiRoot(base);
  try {
    const native = await fetch(`${root}/api/tags`, { signal: AbortSignal.timeout(2000) });
    if (native.ok) {
      const body = (await native.json()) as { models?: { name?: string; model?: string }[] };
      const models = (body.models ?? [])
        .map((m) => m.name || m.model || "")
        .filter(Boolean);
      if (!models.length) {
        return { models: [], ok: false, message: ollamaHelp(base, "empty") };
      }
      return {
        models,
        ok: true,
        message: `Ollama online · ${models.length} model${models.length === 1 ? "" : "s"}`,
      };
    }
  } catch {
    /* try OpenAI-compatible listing next */
  }
  try {
    const models = await listModelsAt(openaiV1(base));
    if (models.length) {
      return {
        models,
        ok: true,
        message: `Ollama online · ${models.length} model${models.length === 1 ? "" : "s"}`,
      };
    }
    const ping = await fetch(`${root}/api/tags`, { signal: AbortSignal.timeout(1500) });
    if (ping.ok) return { models: [], ok: false, message: ollamaHelp(base, "empty") };
  } catch {
    /* offline */
  }
  return { models: [], ok: false, message: ollamaHelp(base, "offline") };
}

export async function listOllamaModels(): Promise<OllamaProbe> {
  return probeOllama(await resolveBase("ollama"));
}
