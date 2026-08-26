import { randomUUID } from "node:crypto";

export type FormatToolCall = {
  id: string;
  type: "function";
  function: { name: string; arguments: string };
};

export type FormatMessage = {
  role: string;
  content: string | null;
  name?: string;
  tool_call_id?: string;
  tool_calls?: FormatToolCall[];
};

export type FormatOpenAiTool = {
  type: "function";
  function: { name: string; description: string; parameters: Record<string, unknown> };
};

export function trimSlash(url: string): string {
  return url.replace(/\/+$/, "");
}

export function anthropicMessagesUrl(base: string): string {
  const t = trimSlash(base);
  if (/\/v1\/messages$/i.test(t)) return t;
  if (/\/v1$/i.test(t)) return `${t}/messages`;
  return `${t}/v1/messages`;
}

export function anthropicModelsUrl(base: string): string {
  const t = trimSlash(base);
  if (/\/v1\/models$/i.test(t)) return t;
  if (/\/v1$/i.test(t)) return `${t}/models`;
  return `${t}/v1/models`;
}

export function geminiGenerateUrl(base: string, model: string): string {
  const t = trimSlash(base);
  if (/:generateContent$/i.test(t)) return t;
  const m = encodeURIComponent(model);
  if (/\/v1beta$/i.test(t) || /\/v1$/i.test(t)) return `${t}/models/${m}:generateContent`;
  return `${t}/v1beta/models/${m}:generateContent`;
}

export function geminiModelsUrl(base: string): string {
  const t = trimSlash(base);
  if (/\/models$/i.test(t)) return t;
  if (/\/v1beta$/i.test(t) || /\/v1$/i.test(t)) return `${t}/models`;
  return `${t}/v1beta/models`;
}

export function azureChatUrl(base: string, deployment: string, apiVersion: string): string {
  const t = trimSlash(base);
  if (/\/chat\/completions/i.test(t)) {
    try {
      const u = new URL(t);
      if (!u.searchParams.get("api-version")) u.searchParams.set("api-version", apiVersion);
      return u.toString();
    } catch {
      return t;
    }
  }
  if (/\/openai\/v1$/i.test(t) || /\/v1$/i.test(t)) {
    const u = new URL(`${t}/chat/completions`);
    u.searchParams.set("api-version", apiVersion);
    return u.toString();
  }
  const dep = encodeURIComponent(deployment || "gpt-4o");
  const u = new URL(`${t}/openai/deployments/${dep}/chat/completions`);
  u.searchParams.set("api-version", apiVersion);
  return u.toString();
}

export function azureDeploymentsUrl(base: string, apiVersion: string): string {
  const t = trimSlash(base);
  const u = new URL(`${t}/openai/deployments`);
  u.searchParams.set("api-version", apiVersion);
  return u.toString();
}

export function openaiToolsToAnthropic(tools: FormatOpenAiTool[]) {
  return tools.map((t) => ({
    name: t.function.name,
    description: t.function.description,
    input_schema: t.function.parameters,
  }));
}

export function openaiToolsToGemini(tools: FormatOpenAiTool[]) {
  return [
    {
      function_declarations: tools.map((t) => ({
        name: t.function.name,
        description: t.function.description,
        parameters: t.function.parameters,
      })),
    },
  ];
}

export function toAnthropicPayload(messages: FormatMessage[]): {
  system: string;
  messages: Record<string, unknown>[];
} {
  const system = messages
    .filter((m) => m.role === "system")
    .map((m) => m.content ?? "")
    .filter(Boolean)
    .join("\n\n");
  const out: Record<string, unknown>[] = [];
  const rest = messages.filter((m) => m.role !== "system");
  let i = 0;
  while (i < rest.length) {
    const m = rest[i];
    if (m.role === "tool") {
      const blocks: unknown[] = [];
      while (i < rest.length && rest[i].role === "tool") {
        blocks.push({
          type: "tool_result",
          tool_use_id: rest[i].tool_call_id || "",
          content: rest[i].content ?? "",
        });
        i += 1;
      }
      out.push({ role: "user", content: blocks });
      continue;
    }
    if (m.role === "assistant" && m.tool_calls?.length) {
      const content: unknown[] = [];
      if (m.content) content.push({ type: "text", text: m.content });
      for (const call of m.tool_calls) {
        let input: unknown = {};
        try {
          input = JSON.parse(call.function.arguments || "{}");
        } catch {
          input = {};
        }
        content.push({
          type: "tool_use",
          id: call.id || randomUUID(),
          name: call.function.name,
          input,
        });
      }
      out.push({ role: "assistant", content });
      i += 1;
      continue;
    }
    out.push({
      role: m.role === "assistant" ? "assistant" : "user",
      content: m.content ?? "",
    });
    i += 1;
  }
  return { system, messages: out };
}

export function fromAnthropicResponse(json: {
  content?: { type?: string; text?: string; id?: string; name?: string; input?: unknown }[];
  stop_reason?: string;
}): { content: string; tool_calls: FormatToolCall[] } {
  const blocks = json.content ?? [];
  const text = blocks
    .filter((b) => b.type === "text")
    .map((b) => b.text ?? "")
    .join("");
  const tool_calls: FormatToolCall[] = blocks
    .filter((b) => b.type === "tool_use" && b.name)
    .map((b) => ({
      id: b.id || randomUUID(),
      type: "function" as const,
      function: { name: b.name as string, arguments: JSON.stringify(b.input ?? {}) },
    }));
  return { content: text, tool_calls };
}

export function toGeminiPayload(messages: FormatMessage[]): {
  system_instruction?: { parts: { text: string }[] };
  contents: Record<string, unknown>[];
} {
  const system = messages
    .filter((m) => m.role === "system")
    .map((m) => m.content ?? "")
    .filter(Boolean)
    .join("\n\n");
  const contents: Record<string, unknown>[] = [];
  const rest = messages.filter((m) => m.role !== "system");
  let i = 0;
  while (i < rest.length) {
    const m = rest[i];
    if (m.role === "tool") {
      const parts: unknown[] = [];
      while (i < rest.length && rest[i].role === "tool") {
        let response: unknown = rest[i].content;
        try {
          response = JSON.parse(rest[i].content || "{}");
        } catch {
          response = { result: rest[i].content };
        }
        parts.push({
          functionResponse: {
            name: rest[i].name || "tool",
            response: typeof response === "object" && response ? response : { result: response },
          },
        });
        i += 1;
      }
      contents.push({ role: "user", parts });
      continue;
    }
    if (m.role === "assistant" && m.tool_calls?.length) {
      const parts: unknown[] = [];
      if (m.content) parts.push({ text: m.content });
      for (const call of m.tool_calls) {
        let args: unknown = {};
        try {
          args = JSON.parse(call.function.arguments || "{}");
        } catch {
          args = {};
        }
        parts.push({ functionCall: { name: call.function.name, args } });
      }
      contents.push({ role: "model", parts });
      i += 1;
      continue;
    }
    contents.push({
      role: m.role === "assistant" ? "model" : "user",
      parts: [{ text: m.content ?? "" }],
    });
    i += 1;
  }
  return {
    ...(system ? { system_instruction: { parts: [{ text: system }] } } : {}),
    contents,
  };
}

export function fromGeminiResponse(json: {
  candidates?: {
    content?: {
      parts?: {
        text?: string;
        functionCall?: { name?: string; args?: unknown };
      }[];
    };
  }[];
}): { content: string; tool_calls: FormatToolCall[] } {
  const parts = json.candidates?.[0]?.content?.parts ?? [];
  const text = parts.map((p) => p.text ?? "").join("");
  const tool_calls: FormatToolCall[] = [];
  for (const p of parts) {
    if (p.functionCall?.name) {
      tool_calls.push({
        id: randomUUID(),
        type: "function",
        function: {
          name: p.functionCall.name,
          arguments: JSON.stringify(p.functionCall.args ?? {}),
        },
      });
    }
  }
  return { content: text, tool_calls };
}
