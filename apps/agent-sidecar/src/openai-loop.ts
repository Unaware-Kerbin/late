import { randomUUID } from "node:crypto";
import { daemon } from "./daemon.js";
import { auditEvent } from "./local-auth.js";
import { loadProviderKeys } from "./provider-keys.js";
import { executeTool, liveSessionContext, OPENAI_TOOLS, type ToolCtx } from "./tools.js";
import {
  LLAMACPP_BASE,
  MAX_ROUNDS,
  OLLAMA_BASE,
  SYSTEM_PROMPT,
  TURN_TIMEOUT_MS,
  VLLM_BASE,
  type ChatMessage,
  type SseEvent,
  type ToolCall,
} from "./types.js";

type CompatKind = "vllm" | "ollama" | "llamacpp";

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
  if (kind === "llamacpp") {
    if (process.env.LATE_LLAMACPP_BASE?.trim()) return openaiV1(process.env.LATE_LLAMACPP_BASE);
    const s = await loadAppSettings();
    return openaiV1(settingStr(s, "llamaCppBaseUrl", "llama_cpp_base_url") ?? LLAMACPP_BASE);
  }
  if (process.env.LATE_VLLM_BASE?.trim()) return openaiV1(process.env.LATE_VLLM_BASE);
  const s = await loadAppSettings();
  return openaiV1(settingStr(s, "vllmBaseUrl", "vllm_base_url") ?? VLLM_BASE);
}

function ollamaHelp(base: string, kind: "offline" | "empty"): string {
  const root = openaiRoot(base);
  if (kind === "empty") {
    return `Ollama is running at ${root} but has no models. Pull one from the Agent pane (for example qwen2.5:7b or Qwen/Qwen3-8B-GGUF).`;
  }
  return `Ollama is not running at ${root}. Install from https://ollama.com and start it, then Pull a model from the Agent pane. Late does not install Ollama.`;
}

function llamaCppHelp(base: string, kind: "offline" | "empty" | "loading" | "auth"): string {
  const root = openaiRoot(base);
  if (kind === "empty") {
    return `llama.cpp is running at ${root} but listed no models. Pass \`-m /path/to/model.gguf\` to llama-server, or Download a GGUF from the Agent pane and Start.`;
  }
  if (kind === "loading") {
    return `llama.cpp is starting at ${root} (model still loading). Wait until llama-server /health is ready, then retry.`;
  }
  if (kind === "auth") {
    return `llama.cpp at ${root} requires an API key (HTTP 401/403). Late does not send cloud provider keys to llama.cpp. Run llama-server without --api-key on loopback, or use a server that does not require a key.`;
  }
  return `llama.cpp (llama-server) is not running at ${root}. Download a GGUF from the Agent pane and Start, or run \`llama-server -m /path/to/model.gguf --port 8080 --host 127.0.0.1\`. Late does not install llama.cpp.`;
}

function compatLabel(kind: CompatKind): string {
  if (kind === "ollama") return "Ollama";
  if (kind === "llamacpp") return "llama.cpp";
  return "vLLM";
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

export async function runLlamaCppChat(opts: {
  messages: ChatMessage[];
  model?: string;
  conversationId: string;
  emit: (e: SseEvent) => void;
  signal: AbortSignal;
}): Promise<string> {
  return runCompatChat({ ...opts, kind: "llamacpp" });
}

async function runCompatChat(opts: {
  kind: CompatKind;
  messages: ChatMessage[];
  model?: string;
  conversationId: string;
  emit: (e: SseEvent) => void;
  signal: AbortSignal;
}): Promise<string> {
  const label = compatLabel(opts.kind);
  const base = await resolveBase(opts.kind);
  await assertChatAllowed(opts.kind, base);
  if (opts.kind === "ollama") {
    const probe = await probeOllama(base);
    if (!probe.ok) throw new Error(probe.message);
  }
  if (opts.kind === "llamacpp") {
    const probe = await probeLlamaCpp(base);
    if (!probe.ok) throw new Error(probe.message);
  }
  if (opts.kind === "vllm") {
    const listed = await listModelsAt("vllm", base);
    if (listed.authFailed) {
      throw new Error(`${label} at ${base} returned ${listed.status} — check the API key, if any.`);
    }
    if (!listed.models.length && listed.status === 0) {
      throw new Error(
        `${label} is not reachable at ${base}. If you just clicked Start, wait until GPU status is running — first load can take several minutes. Do not click Stop while it is starting.`,
      );
    }
  }
  const model = opts.model || (await defaultModel(opts.kind, base));
  const live = await liveSessionContext();
  const messages: ChatMessage[] = [
    { role: "system", content: SYSTEM_PROMPT },
    { role: "system", content: live },
    ...opts.messages.filter((m) => m.role !== "system"),
  ];

  let assistantText = "";
  let gathered = "";
  for (let round = 1; round <= MAX_ROUNDS; round++) {
    if (opts.signal.aborted) throw new Error("stopped");
    opts.emit({ type: "round", n: round, max: MAX_ROUNDS });
    // Do not re-attach full live scrollback every round — that plus command
    // output blows small vLLM contexts and kills the loop after one tool.

    let completion: Awaited<ReturnType<typeof chatCompletions>>;
    try {
      const beat = setInterval(() => {
        opts.emit({
          type: "heartbeat",
          message: gathered ? `Waiting on ${label} after the command…` : `${label} is thinking…`,
        });
      }, 5000);
      try {
        completion = await chatCompletions(opts.kind, label, base, model, messages, opts.signal);
      } catch (err) {
        throw describeFetchError(err, label, base);
      } finally {
        clearInterval(beat);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      if (gathered) {
        const note = `\n\n(model follow-up failed after a command ran: ${msg}. Answering from the output already gathered.)`;
        opts.emit({ type: "delta", text: note });
        return `${assistantText}${note}\n\n${gathered.slice(-4000)}`;
      }
      throw err;
    }
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

    const lastUser = [...opts.messages].reverse().find((m) => m.role === "user");
    const ctx: ToolCtx = {
      conversationId: opts.conversationId,
      emit: opts.emit,
      signal: opts.signal,
      operatorText: (lastUser?.content ?? "").trim(),
    };
    for (const call of toolCalls) {
      const result = clipToolResult(
        await executeTool(call.function.name, call.function.arguments ?? "{}", ctx),
      );
      if (/"executed"\s*:\s*true/.test(result)) gathered = result;
      messages.push({
        role: "tool",
        tool_call_id: call.id || randomUUID(),
        content: result,
      });
    }
  }
  throw new Error(`agent hit the ${MAX_ROUNDS}-round cap`);
}

function clipToolResult(s: string, n = 4000): string {
  if (s.length <= n) return s;
  return `${s.slice(0, 1200)}\n…[truncated ${s.length - n} chars]…\n${s.slice(-(n - 1400))}`;
}

async function defaultModel(kind: CompatKind, base: string): Promise<string> {
  if (kind === "ollama") {
    const probe = await probeOllama(base);
    const s = await loadAppSettings();
    const preferred = settingStr(s, "ollamaModel", "ollama_model");
    if (preferred && probe.models.includes(preferred)) return preferred;
    return probe.models[0] ?? "llama3.2";
  }
  const models = (await listModelsAt(kind, base)).models;
  const s = await loadAppSettings();
  if (kind === "llamacpp") {
    const preferred = settingStr(s, "llamaCppModel", "llama_cpp_model");
    if (preferred && models.includes(preferred)) return preferred;
    return models[0] ?? preferred ?? "local";
  }
  const preferred = settingStr(s, "vllmModel", "vllm_model");
  if (preferred && models.includes(preferred)) return preferred;
  return models[0] ?? "local";
}

async function chatCompletions(
  kind: CompatKind,
  label: string,
  base: string,
  model: string,
  messages: ChatMessage[],
  signal: AbortSignal,
) {
  const linked = AbortSignal.any([signal, AbortSignal.timeout(TURN_TIMEOUT_MS)]);
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    ...(await bearerFor(kind, base)),
  };
  let r: Response;
  try {
    r = await fetchStayOnBox(`${base}/chat/completions`, {
      method: "POST",
      headers,
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
  } catch (err) {
    throw describeFetchError(err, label, base);
  }
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

function isLoopbackBase(base: string): boolean {
  try {
    return isLoopbackUrl(new URL(base));
  } catch {
    return false;
  }
}

function isLoopbackUrl(u: URL): boolean {
  if (u.protocol !== "http:" && u.protocol !== "https:") return false;
  const host = u.hostname.replace(/^\[|\]$/g, "").toLowerCase();
  return host === "127.0.0.1" || host === "localhost" || host === "::1";
}

/** Follow one same-host loopback redirect (trailing slash). Refuse off-box 302s. */
async function fetchStayOnBox(url: string, init: RequestInit): Promise<Response> {
  const first = await fetch(url, { ...init, redirect: "manual" });
  if (first.status < 300 || first.status >= 400) return first;
  const loc = first.headers.get("location");
  if (!loc) {
    throw new Error("redirect without Location");
  }
  const orig = new URL(url);
  const next = new URL(loc, url);
  const sameHost =
    orig.protocol === next.protocol &&
    orig.hostname === next.hostname &&
    orig.port === next.port;
  if (!sameHost || (isLoopbackUrl(orig) && !isLoopbackUrl(next))) {
    throw new Error("refusing off-box or cross-host chat redirect");
  }
  return fetch(next.toString(), { ...init, redirect: "error" });
}

function settingFlag(s: Record<string, unknown>, ...keys: string[]): boolean {
  for (const k of keys) {
    if (s[k] === true) return true;
  }
  return false;
}

async function cloudChatEnabled(): Promise<boolean> {
  settingsCache = null;
  const s = await loadAppSettings();
  return settingFlag(s, "cloud_chat_enabled", "cloudChatEnabled");
}

const CLOUD_OFF =
  "Cloud AI is off. Turn it on in Settings. Session text may then leave your computer.";

export async function assertChatAllowed(
  kind: "vllm" | "ollama" | "llamacpp" | "cursor",
  base?: string,
): Promise<void> {
  const cloud = await cloudChatEnabled();
  if (kind === "cursor") {
    auditEvent("chat", { backend: "cursor", egress: "cloud", ok: cloud });
    if (!cloud) throw new Error(CLOUD_OFF);
    return;
  }
  const loop = Boolean(base && isLoopbackBase(base));
  const ok = loop || cloud;
  auditEvent("chat", { backend: kind, egress: loop ? "loopback" : "cloud", ok });
  if (!ok) {
    throw new Error(
      `${compatLabel(kind)} at ${base} is not loopback. Turn on Cloud AI in Settings to send session text off your computer.`,
    );
  }
}

function describeFetchError(err: unknown, label: string, base: string): Error {
  const cause =
    err instanceof Error ? (err as Error & { cause?: { code?: string } }).cause : undefined;
  const code = cause?.code ?? "";
  const msg = err instanceof Error ? err.message : String(err);
  if (
    code === "ECONNREFUSED" ||
    code === "ECONNRESET" ||
    code === "ETIMEDOUT" ||
    code === "UND_ERR_CONNECT_TIMEOUT" ||
    code === "UND_ERR_SOCKET" ||
    /fetch failed/i.test(msg)
  ) {
    return new Error(
      `${label} is not reachable at ${base}. If you just clicked Start, wait until GPU status is running — first load can take several minutes. Do not click Stop while it is starting.`,
    );
  }
  return err instanceof Error ? err : new Error(`${label}: ${msg}`);
}

async function bearerFor(kind: CompatKind, base: string): Promise<Record<string, string>> {
  if (kind === "ollama" || kind === "llamacpp") return {};
  if (isLoopbackBase(base)) return {};
  if (!(await cloudChatEnabled())) return {};
  const keys = await loadProviderKeys();
  let host = "";
  try {
    host = new URL(base).hostname.replace(/^\[|\]$/g, "").toLowerCase();
  } catch {
    return {};
  }
  let key = "";
  if (host === "api.openai.com" || host.endsWith(".openai.com")) {
    key = keys.openai ?? "";
  } else if (host === "openrouter.ai" || host.endsWith(".openrouter.ai")) {
    key = keys.openrouter ?? "";
  } else if (host === "api.groq.com" || host.endsWith(".groq.com")) {
    key = keys.groq ?? "";
  } else {
    key = keys.custom ?? "";
  }
  return key ? { Authorization: `Bearer ${key}` } : {};
}

type ModelList = { models: string[]; authFailed: boolean; status: number };

async function listModelsAt(kind: CompatKind, base: string): Promise<ModelList> {
  try {
    const headers = await bearerFor(kind, base);
    const r = await fetchStayOnBox(`${base}/models`, {
      headers,
      signal: AbortSignal.timeout(2000),
    });
    if (r.status === 401 || r.status === 403) {
      return { models: [], authFailed: true, status: r.status };
    }
    if (!r.ok) return { models: [], authFailed: false, status: r.status };
    const body = (await r.json()) as { data?: { id: string }[] };
    return {
      models: (body.data ?? []).map((m) => m.id).filter((id) => !id.includes(":cloud")),
      authFailed: false,
      status: r.status,
    };
  } catch {
    return { models: [], authFailed: false, status: 0 };
  }
}

export async function listLocalModels(): Promise<string[]> {
  return (await listModelsAt("vllm", await resolveBase("vllm"))).models;
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
    const listed = await listModelsAt("ollama", openaiV1(base));
    if (listed.models.length) {
      return {
        models: listed.models,
        ok: true,
        message: `Ollama online · ${listed.models.length} model${listed.models.length === 1 ? "" : "s"}`,
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

async function probeLlamaCpp(base: string): Promise<OllamaProbe> {
  const listed = await listModelsAt("llamacpp", openaiV1(base));
  if (listed.authFailed) {
    return { models: [], ok: false, message: llamaCppHelp(base, "auth") };
  }
  if (listed.models.length) {
    return {
      models: listed.models,
      ok: true,
      message: `llama.cpp online · ${listed.models.length} model${listed.models.length === 1 ? "" : "s"}`,
    };
  }
  try {
    const ping = await fetch(`${openaiRoot(base)}/health`, {
      headers: await bearerFor("llamacpp", base),
      signal: AbortSignal.timeout(1500),
    });
    if (ping.status === 401 || ping.status === 403) {
      return { models: [], ok: false, message: llamaCppHelp(base, "auth") };
    }
    if (ping.status === 503) {
      return { models: [], ok: false, message: llamaCppHelp(base, "loading") };
    }
    if (ping.ok) return { models: [], ok: false, message: llamaCppHelp(base, "empty") };
  } catch {
    /* offline */
  }
  return { models: [], ok: false, message: llamaCppHelp(base, "offline") };
}

export async function listLlamaCppModels(): Promise<OllamaProbe> {
  return probeLlamaCpp(await resolveBase("llamacpp"));
}
