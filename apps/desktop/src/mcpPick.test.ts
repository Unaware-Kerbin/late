import assert from "node:assert/strict";
import test from "node:test";
import {
  backendLabel,
  coerceChatBackend,
  DEFAULT_MCP_HTTP_URL,
  LOCAL_INFERENCE_URL,
  mcpLocationLabel,
  mcpPickLabel,
  mcpPickMode,
  mcpStatusLine,
  withMcpPick,
  type AppSettings,
} from "./types.ts";

function base(): AppSettings {
  return {
    bind: "127.0.0.1:7420",
    vllm_base_url: "http://127.0.0.1:8000/v1",
    vllm_model: "local",
    cursor_model: "composer-2.5",
    ollama_base_url: "http://127.0.0.1:11434/v1",
    ollama_model: "",
    llama_cpp_base_url: "http://127.0.0.1:8080/v1",
    llama_cpp_model: "",
    anthropic_base_url: "https://api.anthropic.com",
    anthropic_model: "claude-sonnet-4-5",
    gemini_base_url: "https://generativelanguage.googleapis.com",
    gemini_model: "gemini-2.5-flash",
    azure_base_url: "",
    azure_deployment: "",
    azure_api_version: "2024-10-21",
    default_backend: "local",
    scrollback_lines: 32_000,
    turn_timeout_secs: 90,
    max_agent_rounds: 50,
    mcp_enabled: false,
    mcp_cwd: "",
    mcp_url: "",
  };
}

test("MCP picker defaults to off", () => {
  assert.equal(mcpPickMode(null), "off");
  assert.equal(mcpPickMode(base()), "off");
  assert.equal(mcpPickLabel(base()), "MCP off");
  assert.equal(base().default_backend, "local");
  assert.equal(coerceChatBackend(base().default_backend), "local");
  assert.equal(mcpPickMode({ ...base(), mcp_url: DEFAULT_MCP_HTTP_URL }), "off");
  assert.equal(DEFAULT_MCP_HTTP_URL, "http://127.0.0.1:8790/mcp");
  assert.doesNotMatch(DEFAULT_MCP_HTTP_URL, /8787/);
  assert.doesNotMatch(DEFAULT_MCP_HTTP_URL, /8000/);
  assert.notEqual(DEFAULT_MCP_HTTP_URL, LOCAL_INFERENCE_URL.local);
});

test("This computer is stdio when MCP is on and the URL is empty", () => {
  const s = withMcpPick(base(), { enabled: true, url: "" });
  assert.equal(s.mcp_enabled, true);
  assert.equal(s.mcp_url, "");
  assert.equal(mcpPickMode(s), "stdio");
  assert.equal(mcpPickLabel(s), "MCP · this computer");
});

test("HTTP address uses mcp_url and is not a model id", () => {
  const s = withMcpPick(base(), { enabled: true, url: DEFAULT_MCP_HTTP_URL });
  assert.equal(mcpPickMode(s), "http");
  assert.equal(mcpPickLabel(s), "MCP · 127.0.0.1:8790");
  assert.equal(mcpLocationLabel(s), "127.0.0.1:8790");
  const off = withMcpPick(s, { enabled: false, url: s.mcp_url ?? "" });
  assert.equal(mcpPickMode(off), "off");
  assert.equal(off.mcp_url, DEFAULT_MCP_HTTP_URL);
});

test("MCP is a chat backend like vLLM, not a model id", () => {
  assert.equal(coerceChatBackend("mcp"), "mcp");
  assert.equal(backendLabel("mcp"), "MCP");
  assert.equal(coerceChatBackend("unknown"), "local");
  const loc = withMcpPick(base(), { enabled: true, url: "" });
  assert.equal(mcpLocationLabel(loc), "This computer");
});

test("selecting MCP is explicit and does not retarget Late vLLM :8000", () => {
  const off = base();
  assert.equal(off.mcp_enabled, false);
  assert.equal(off.default_backend, "local");
  const on = withMcpPick(off, { enabled: true, url: DEFAULT_MCP_HTTP_URL });
  assert.equal(on.mcp_enabled, true);
  assert.equal(on.default_backend, "local");
  assert.equal(on.vllm_base_url, "http://127.0.0.1:8000/v1");
  assert.equal(coerceChatBackend("mcp"), "mcp");
  assert.notEqual(coerceChatBackend("mcp"), "local");
  assert.equal(coerceChatBackend("cursor"), "cursor");
});

test("status is MCP on when tools/list works; GUI 401 is not MCP down", () => {
  assert.equal(mcpStatusLine({ enabled: false, ok: true, tools: [] }), "MCP off");
  assert.equal(
    mcpStatusLine({ enabled: true, ok: true, tools: ["mcp_list_agents", "mcp_chat_send"] }),
    "MCP on · 2 tools",
  );
  assert.equal(mcpStatusLine({ enabled: true, ok: true, tools: ["mcp_list_agents"] }), "MCP on · 1 tool");
  assert.equal(
    mcpStatusLine({ enabled: true, ok: false, message: "MCP HTTP 401: unauthorized" }),
    "MCP error",
  );
  assert.equal(
    mcpStatusLine({ enabled: true, ok: true, tools: ["mcp_list_agents"], message: "GUI logos unavailable" }),
    "MCP on · 1 tool",
  );
  assert.equal(
    mcpStatusLine({ enabled: true, ok: false, message: "GUI logos unavailable (8787)" }),
    "GUI logos unavailable",
  );
  assert.equal(
    mcpStatusLine({ enabled: true, ok: false, message: "ECONNREFUSED 127.0.0.1:8787" }),
    "MCP error",
  );
  assert.equal(
    mcpStatusLine({ enabled: true, ok: false, message: "MCP HTTP 404 at http://127.0.0.1:8787/mcp" }),
    "MCP error",
  );
});
