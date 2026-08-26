import { randomUUID } from "node:crypto";
import {
  classifyChatBase,
  fetchStayOnBox,
  parseExtraHosts,
  type ChatEgress,
} from "./chat-target.js";
import { auditEvent } from "./local-auth.js";
import {
  assertChatAllowed,
  loadAppSettings,
  settingStr,
  type ChatProbe,
  type ChatTargetMeta,
} from "./openai-loop.js";
import { loadProviderKeys } from "./provider-keys.js";
import { executeTool, liveSessionContext, OPENAI_TOOLS, type ToolCtx } from "./tools.js";
import {
  MAX_ROUNDS,
  SYSTEM_PROMPT,
  TURN_TIMEOUT_MS,
  type ChatMessage,
  type SseEvent,
  type ToolCall,
} from "./types.js";
import {
  anthropicMessagesUrl,
  anthropicModelsUrl,
  azureChatUrl,
  azureDeploymentsUrl,
  fromAnthropicResponse,
  fromGeminiResponse,
  geminiGenerateUrl,
  geminiModelsUrl,
  openaiToolsToAnthropic,
  openaiToolsToGemini,
  toAnthropicPayload,
  toGeminiPayload,
  trimSlash,
} from "./native-format.js";

export type NativeKind = "anthropic" | "gemini" | "azure";

export const ANTHROPIC_MODELS = ["claude-sonnet-4-5", "claude-opus-4-5", "claude-haiku-4-5"];
export const GEMINI_MODELS = ["gemini-2.5-flash", "gemini-2.5-pro", "gemini-2.0-flash"];

export {
  anthropicMessagesUrl,
  anthropicModelsUrl,
  azureChatUrl,
  azureDeploymentsUrl,
  fromAnthropicResponse,
  fromGeminiResponse,
  geminiGenerateUrl,
  geminiModelsUrl,
  openaiToolsToAnthropic,
  openaiToolsToGemini,
  toAnthropicPayload,
  toGeminiPayload,
  trimSlash,
} from "./native-format.js";

const ANTHROPIC_BASE = "https://api.anthropic.com";
const GEMINI_BASE = "https://generativelanguage.googleapis.com";
const AZURE_API_VERSION = "2024-10-21";

type CompleteFn = (
  messages: ChatMessage[],
  signal: AbortSignal,
) => Promise<{ content: string; tool_calls: ToolCall[] }>;

function clipToolResult(s: string, n = 4000): string {
  if (s.length <= n) return s;
  return `${s.slice(0, 1200)}\n…[truncated ${s.length - n} chars]…\n${s.slice(-(n - 1400))}`;
}

async function runToolLoop(opts: {
  label: string;
  messages: ChatMessage[];
  conversationId: string;
  emit: (e: SseEvent) => void;
  signal: AbortSignal;
  complete: CompleteFn;
}): Promise<string> {
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
    let done: { content: string; tool_calls: ToolCall[] };
    const beat = setInterval(() => {
      opts.emit({
        type: "heartbeat",
        message: gathered ? `Waiting on ${opts.label} after the command…` : `${opts.label} is thinking…`,
      });
    }, 5000);
    try {
      done = await opts.complete(messages, opts.signal);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      if (gathered) {
        const note = `\n\n(model follow-up failed after a command ran: ${msg}. Answering from the output already gathered.)`;
        opts.emit({ type: "delta", text: note });
        return `${assistantText}${note}\n\n${gathered.slice(-4000)}`;
      }
      throw err;
    } finally {
      clearInterval(beat);
    }
    if (done.content) {
      opts.emit({ type: "delta", text: done.content });
      assistantText += done.content;
    }
    if (!done.tool_calls.length) return assistantText;
    messages.push({
      role: "assistant",
      content: done.content || null,
      tool_calls: done.tool_calls,
    });
    const lastUser = [...opts.messages].reverse().find((m) => m.role === "user");
    const ctx: ToolCtx = {
      conversationId: opts.conversationId,
      emit: opts.emit,
      signal: opts.signal,
      operatorText: (lastUser?.content ?? "").trim(),
    };
    for (const call of done.tool_calls) {
      const result = clipToolResult(
        await executeTool(call.function.name, call.function.arguments ?? "{}", ctx),
      );
      if (/"executed"\s*:\s*true/.test(result)) gathered = result;
      messages.push({
        role: "tool",
        name: call.function.name,
        tool_call_id: call.id || randomUUID(),
        content: result,
      });
    }
  }
  throw new Error(`agent hit the ${MAX_ROUNDS}-round cap`);
}

async function vendorHeaders(kind: NativeKind): Promise<Record<string, string>> {
  const keys = await loadProviderKeys();
  if (kind === "anthropic") {
    const key = process.env.ANTHROPIC_API_KEY || keys.anthropic || keys.custom || "";
    return key
      ? { "x-api-key": key, "anthropic-version": "2023-06-01", "Content-Type": "application/json" }
      : { "Content-Type": "application/json", "anthropic-version": "2023-06-01" };
  }
  if (kind === "gemini") {
    const key = process.env.GEMINI_API_KEY || process.env.GOOGLE_API_KEY || keys.gemini || keys.custom || "";
    return key
      ? { "x-goog-api-key": key, "Content-Type": "application/json" }
      : { "Content-Type": "application/json" };
  }
  const key = process.env.AZURE_OPENAI_API_KEY || keys.azure || keys.custom || "";
  if (key) return { "api-key": key, "Content-Type": "application/json" };
  return { "Content-Type": "application/json" };
}

async function resolveNative(kind: NativeKind): Promise<{
  base: string;
  model: string;
  deployment: string;
  apiVersion: string;
}> {
  const s = await loadAppSettings();
  if (kind === "anthropic") {
    const base =
      process.env.LATE_ANTHROPIC_BASE?.trim() ||
      settingStr(s, "anthropic_base_url", "anthropicBaseUrl") ||
      ANTHROPIC_BASE;
    const model =
      settingStr(s, "anthropic_model", "anthropicModel") || ANTHROPIC_MODELS[0];
    return { base: trimSlash(base), model, deployment: "", apiVersion: "" };
  }
  if (kind === "gemini") {
    const base =
      process.env.LATE_GEMINI_BASE?.trim() ||
      settingStr(s, "gemini_base_url", "geminiBaseUrl") ||
      GEMINI_BASE;
    const model = settingStr(s, "gemini_model", "geminiModel") || GEMINI_MODELS[0];
    return { base: trimSlash(base), model, deployment: "", apiVersion: "" };
  }
  const base =
    process.env.LATE_AZURE_BASE?.trim() ||
    settingStr(s, "azure_base_url", "azureBaseUrl") ||
    "";
  const deployment =
    settingStr(s, "azure_deployment", "azureDeployment", "azure_model", "azureModel") || "";
  const apiVersion =
    settingStr(s, "azure_api_version", "azureApiVersion") || AZURE_API_VERSION;
  return { base: trimSlash(base), model: deployment, deployment, apiVersion };
}

async function extraHosts(): Promise<string[]> {
  const s = await loadAppSettings();
  return parseExtraHosts(s.private_inference_hosts ?? s.privateInferenceHosts);
}

async function readJson(url: string, init: RequestInit, label: string): Promise<{ status: number; json: unknown; text: string }> {
  const r = await fetchStayOnBox(url, init);
  const text = await r.text();
  let json: unknown = {};
  try {
    json = text ? JSON.parse(text) : {};
  } catch {
    throw new Error(`${label} returned non-JSON (${r.status}): ${text.slice(0, 240)}`);
  }
  return { status: r.status, json, text };
}

export async function nativeTargetMeta(kind: NativeKind): Promise<ChatTargetMeta> {
  const { base } = await resolveNative(kind);
  const extra = await extraHosts();
  const egress: ChatEgress = base ? await classifyChatBase(base, extra) : "cloud";
  return { base, egress };
}

export async function probeNativeTarget(kind: NativeKind, baseOverride?: string): Promise<ChatProbe> {
  const resolved = await resolveNative(kind);
  const base = trimSlash((baseOverride ?? "").trim() || resolved.base);
  const extra = await extraHosts();
  const egress = base ? await classifyChatBase(base, extra) : "cloud";
  if (!base) {
    const message =
      kind === "azure"
        ? "Set the Azure resource URL (for example https://YOUR_RESOURCE.openai.azure.com or a private endpoint)."
        : `Set the ${kind} base URL in Settings.`;
    auditEvent("chat.probe", { kind, egress, ok: false });
    return { ok: false, kind, base, egress, models: [], message };
  }
  try {
    await assertChatAllowed(kind, base);
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    auditEvent("chat.probe", { kind, egress, ok: false });
    return { ok: false, kind, base, egress, models: [], message };
  }
  const headers = await vendorHeaders(kind);
  const where = egress === "loopback" ? "this computer" : egress === "private" ? "your network" : "public internet";
  try {
    if (kind === "anthropic") {
      const { status, json } = await readJson(
        anthropicModelsUrl(base),
        { headers, signal: AbortSignal.timeout(4000) },
        "Anthropic",
      );
      const models = (
        (json as { data?: { id?: string }[] }).data ?? []
      )
        .map((m) => m.id)
        .filter((id): id is string => Boolean(id));
      const list = models.length ? models : ANTHROPIC_MODELS;
      const ok = status < 400 || models.length > 0;
      const message = ok
        ? `Anthropic online · ${list.length} model${list.length === 1 ? "" : "s"} · ${where}`
        : `Anthropic at ${base} returned HTTP ${status}. Save the Anthropic key, or point at a private /v1/messages proxy.`;
      auditEvent("chat.probe", { kind, egress, ok });
      return { ok, kind, base, egress, models: list, message };
    }
    if (kind === "gemini") {
      const { status, json } = await readJson(
        geminiModelsUrl(base),
        { headers, signal: AbortSignal.timeout(4000) },
        "Gemini",
      );
      const models = ((json as { models?: { name?: string }[] }).models ?? [])
        .map((m) => (m.name ?? "").replace(/^models\//, ""))
        .filter(Boolean)
        .filter((id) => /gemini/i.test(id));
      const list = models.length ? models : GEMINI_MODELS;
      const ok = status < 400 || models.length > 0;
      const message = ok
        ? `Gemini online · ${list.length} model${list.length === 1 ? "" : "s"} · ${where}`
        : `Gemini at ${base} returned HTTP ${status}. Save the Gemini key, or point at a Vertex / generateContent proxy.`;
      auditEvent("chat.probe", { kind, egress, ok });
      return { ok, kind, base, egress, models: list, message };
    }
    const { apiVersion, deployment } = resolved;
    const { status, json } = await readJson(
      azureDeploymentsUrl(base, apiVersion),
      { headers, signal: AbortSignal.timeout(4000) },
      "Azure",
    );
    const models = ((json as { data?: { id?: string }[] }).data ?? [])
      .map((m) => m.id)
      .filter((id): id is string => Boolean(id));
    if (deployment && !models.includes(deployment)) models.unshift(deployment);
    const list = models.length ? models : deployment ? [deployment] : [];
    const ok = status < 400 || list.length > 0;
    const message = ok
      ? `Azure online · ${list.length || 1} deployment${list.length === 1 ? "" : "s"} · ${where}`
      : `Azure at ${base} returned HTTP ${status}. Set the resource URL, deployment name, and Azure key.`;
    auditEvent("chat.probe", { kind, egress, ok });
    return { ok, kind, base, egress, models: list, message };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    auditEvent("chat.probe", { kind, egress, ok: false });
    const fallback =
      kind === "anthropic" ? ANTHROPIC_MODELS : kind === "gemini" ? GEMINI_MODELS : [];
    return { ok: false, kind, base, egress, models: fallback, message };
  }
}

export async function listNativeModels(kind: NativeKind): Promise<string[]> {
  const { model, deployment } = await resolveNative(kind);
  const uniq = (ids: string[]) => [...new Set(ids.filter(Boolean))];
  if (kind === "anthropic") return uniq([model, ...ANTHROPIC_MODELS]);
  if (kind === "gemini") return uniq([model, ...GEMINI_MODELS]);
  return deployment ? [deployment] : [];
}

type NativeChatOpts = {
  messages: ChatMessage[];
  model?: string;
  conversationId: string;
  emit: (e: SseEvent) => void;
  signal: AbortSignal;
};

export async function runAnthropicChat(opts: NativeChatOpts): Promise<string> {
  const { base, model: fallback } = await resolveNative("anthropic");
  await assertChatAllowed("anthropic", base);
  const model = opts.model || fallback;
  const headers = await vendorHeaders("anthropic");
  const url = anthropicMessagesUrl(base);
  return runToolLoop({
    label: "Anthropic",
    messages: opts.messages,
    conversationId: opts.conversationId,
    emit: opts.emit,
    signal: opts.signal,
    complete: async (messages, signal) => {
      const body = toAnthropicPayload(messages);
      const { status, json, text } = await readJson(
        url,
        {
          method: "POST",
          headers,
          signal: AbortSignal.any([signal, AbortSignal.timeout(TURN_TIMEOUT_MS)]),
          body: JSON.stringify({
            model,
            max_tokens: 8192,
            system: body.system,
            messages: body.messages,
            tools: openaiToolsToAnthropic(OPENAI_TOOLS),
            temperature: 0.2,
          }),
        },
        "Anthropic",
      );
      if (status >= 400) {
        throw new Error(`Anthropic ${status}: ${text.slice(0, 400)}`);
      }
      return fromAnthropicResponse(json as Parameters<typeof fromAnthropicResponse>[0]);
    },
  });
}

export async function runGeminiChat(opts: NativeChatOpts): Promise<string> {
  const { base, model: fallback } = await resolveNative("gemini");
  await assertChatAllowed("gemini", base);
  const model = opts.model || fallback;
  const headers = await vendorHeaders("gemini");
  return runToolLoop({
    label: "Gemini",
    messages: opts.messages,
    conversationId: opts.conversationId,
    emit: opts.emit,
    signal: opts.signal,
    complete: async (messages, signal) => {
      const url = geminiGenerateUrl(base, model);
      const payload = toGeminiPayload(messages);
      const { status, json, text } = await readJson(
        url,
        {
          method: "POST",
          headers,
          signal: AbortSignal.any([signal, AbortSignal.timeout(TURN_TIMEOUT_MS)]),
          body: JSON.stringify({
            ...payload,
            tools: openaiToolsToGemini(OPENAI_TOOLS),
            generationConfig: { temperature: 0.2 },
          }),
        },
        "Gemini",
      );
      if (status >= 400) {
        throw new Error(`Gemini ${status}: ${text.slice(0, 400)}`);
      }
      return fromGeminiResponse(json as Parameters<typeof fromGeminiResponse>[0]);
    },
  });
}

export async function runAzureChat(opts: NativeChatOpts): Promise<string> {
  const resolved = await resolveNative("azure");
  if (!resolved.base) {
    throw new Error("Set the Azure resource URL in Settings (public resource or a private endpoint).");
  }
  await assertChatAllowed("azure", resolved.base);
  const deployment = opts.model || resolved.deployment;
  if (!deployment) {
    throw new Error("Set the Azure deployment name in Settings.");
  }
  const headers = await vendorHeaders("azure");
  const url = azureChatUrl(resolved.base, deployment, resolved.apiVersion);
  return runToolLoop({
    label: "Azure",
    messages: opts.messages,
    conversationId: opts.conversationId,
    emit: opts.emit,
    signal: opts.signal,
    complete: async (messages, signal) => {
      const { status, json, text } = await readJson(
        url,
        {
          method: "POST",
          headers,
          signal: AbortSignal.any([signal, AbortSignal.timeout(TURN_TIMEOUT_MS)]),
          body: JSON.stringify({
            messages,
            tools: OPENAI_TOOLS,
            tool_choice: "auto",
            temperature: 0.2,
            stream: false,
          }),
        },
        "Azure",
      );
      if (status >= 400) {
        throw new Error(`Azure ${status}: ${text.slice(0, 400)}`);
      }
      const choice = (json as { choices?: { message?: ChatMessage }[] }).choices?.[0]?.message;
      if (!choice) throw new Error("Azure returned no choices");
      return {
        content: choice.content ?? "",
        tool_calls: choice.tool_calls ?? [],
      };
    },
  });
}
