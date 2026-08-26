import assert from "node:assert/strict";
import test from "node:test";
import {
  anthropicMessagesUrl,
  azureChatUrl,
  fromAnthropicResponse,
  fromGeminiResponse,
  geminiGenerateUrl,
  openaiToolsToAnthropic,
  toAnthropicPayload,
  toGeminiPayload,
} from "./native-format.ts";
import type { ChatMessage, OpenAiTool } from "./types.ts";

const tools: OpenAiTool[] = [
  {
    type: "function",
    function: {
      name: "propose_command",
      description: "Propose a CLI command",
      parameters: { type: "object", properties: { command: { type: "string" } } },
    },
  },
];

test("anthropic and gemini URL builders", () => {
  assert.equal(anthropicMessagesUrl("https://api.anthropic.com"), "https://api.anthropic.com/v1/messages");
  assert.equal(anthropicMessagesUrl("http://10.0.0.9:4000/v1"), "http://10.0.0.9:4000/v1/messages");
  assert.equal(
    geminiGenerateUrl("https://generativelanguage.googleapis.com", "gemini-2.5-flash"),
    "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent",
  );
  assert.equal(
    geminiGenerateUrl("http://10.1.2.3:8080/v1beta", "gemini-2.0-flash"),
    "http://10.1.2.3:8080/v1beta/models/gemini-2.0-flash:generateContent",
  );
});

test("azure deployment path and existing completions URL", () => {
  const a = azureChatUrl("https://demo.openai.azure.com", "gpt-4o", "2024-10-21");
  assert.match(a, /\/openai\/deployments\/gpt-4o\/chat\/completions/);
  assert.match(a, /api-version=2024-10-21/);
  const b = azureChatUrl("http://10.0.0.5/openai/v1", "ignored", "2024-10-21");
  assert.match(b, /\/openai\/v1\/chat\/completions/);
});

test("anthropic tool round-trip", () => {
  const mapped = openaiToolsToAnthropic(tools);
  assert.equal(mapped[0].name, "propose_command");
  assert.equal(mapped[0].input_schema.type, "object");

  const msgs: ChatMessage[] = [
    { role: "system", content: "rules" },
    { role: "user", content: "show bgp" },
    {
      role: "assistant",
      content: null,
      tool_calls: [
        {
          id: "call_1",
          type: "function",
          function: { name: "propose_command", arguments: '{"command":"show bgp"}' },
        },
      ],
    },
    { role: "tool", tool_call_id: "call_1", content: '{"ok":true}' },
  ];
  const payload = toAnthropicPayload(msgs);
  assert.match(payload.system, /rules/);
  assert.equal(payload.messages[0].role, "user");
  const assistant = payload.messages[1] as { role: string; content: { type: string; id?: string }[] };
  assert.equal(assistant.role, "assistant");
  assert.equal(assistant.content.some((b) => b.type === "tool_use" && b.id === "call_1"), true);
  const toolUser = payload.messages[2] as { role: string; content: { type: string }[] };
  assert.equal(toolUser.role, "user");
  assert.equal(toolUser.content[0].type, "tool_result");

  const parsed = fromAnthropicResponse({
    content: [
      { type: "text", text: "next" },
      { type: "tool_use", id: "x", name: "list_open_sessions", input: {} },
    ],
  });
  assert.equal(parsed.content, "next");
  assert.equal(parsed.tool_calls[0].function.name, "list_open_sessions");
});

test("gemini function call parse", () => {
  const msgs: ChatMessage[] = [
    { role: "system", content: "sys" },
    { role: "user", content: "hi" },
  ];
  const payload = toGeminiPayload(msgs);
  assert.equal(payload.system_instruction?.parts[0].text, "sys");
  assert.equal(payload.contents[0].role, "user");

  const parsed = fromGeminiResponse({
    candidates: [
      {
        content: {
          parts: [
            { text: "ok" },
            { functionCall: { name: "propose_command", args: { command: "show ip int br" } } },
          ],
        },
      },
    ],
  });
  assert.equal(parsed.content, "ok");
  assert.equal(parsed.tool_calls[0].function.name, "propose_command");
  assert.match(parsed.tool_calls[0].function.arguments, /show ip int br/);
});
