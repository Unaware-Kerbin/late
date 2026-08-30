import assert from "node:assert/strict";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, it } from "node:test";
import { chatEgressAllowed, classifyChatBaseSync } from "./chat-target.ts";
import {
  mcpNeedsApprove,
  mcpStdioEnv,
  mcpToolName,
  mcpToOpenAiTool,
  parseMcpHttpUrl,
  resolveMcpLaunch,
  resolveMcpTarget,
  splitMcpArgs,
  withStartVllmGpuArgs,
  stripMcpPrefix,
} from "./mcp-format.ts";

/** Operator-chosen folder. Never a machine-specific home path in source. */
const MCP_CWD = join(tmpdir(), "mcp-proj");

describe("mcp-format", () => {
  it("prefixes tool names for Late", () => {
    assert.equal(mcpToolName("list_agents"), "mcp_list_agents");
    assert.equal(mcpToolName("mcp_list_agents"), "mcp_list_agents");
    assert.equal(stripMcpPrefix("mcp_start_vllm"), "start_vllm");
  });

  it("gates writes and starts, not named status/list tools", () => {
    assert.equal(mcpNeedsApprove("list_agents"), false);
    assert.equal(mcpNeedsApprove("mcp_list_hardware"), false);
    assert.equal(mcpNeedsApprove("vllm_status"), false);
    assert.equal(mcpNeedsApprove("ollama_status"), false);
    assert.equal(mcpNeedsApprove("chat_get"), false);
    assert.equal(mcpNeedsApprove("get_run"), false);
    assert.equal(mcpNeedsApprove("start_vllm"), true);
    assert.equal(mcpNeedsApprove("chat_send"), true);
    assert.equal(mcpNeedsApprove("dispatch"), true);
    assert.equal(mcpNeedsApprove("follow_up"), true);
    assert.equal(mcpNeedsApprove("download_local_model"), true);
    assert.equal(mcpNeedsApprove("add_allowed_dir"), true);
    assert.equal(mcpNeedsApprove("chat_delete"), true);
    assert.equal(mcpNeedsApprove("get_secrets"), true);
    assert.equal(mcpNeedsApprove("get_env"), true);
    assert.equal(mcpNeedsApprove("list_passwords"), true);
    assert.equal(mcpNeedsApprove("list_keys"), true);
    assert.equal(mcpNeedsApprove("recommend_exploit"), true);
    assert.equal(mcpNeedsApprove("list_allowed_dirs"), false);
    assert.equal(mcpNeedsApprove("list_local_models"), false);
    assert.equal(mcpNeedsApprove("recommend_local_models"), false);
    assert.equal(mcpNeedsApprove("list_runs"), false);
    assert.equal(mcpNeedsApprove("chat_list"), false);
    assert.equal(mcpNeedsApprove("llamacpp_status"), false);
    assert.equal(mcpNeedsApprove("stop_vllm"), true);
    assert.equal(mcpNeedsApprove("chat_approve"), true);
  });

  it("rejects shell characters in args", () => {
    const bad = splitMcpArgs("src/index.ts; rm -rf /");
    assert.ok("error" in bad);
  });

  it("resolves tsx + src for an orchestrator folder", () => {
    const cwd = MCP_CWD;
    const launch = resolveMcpLaunch(
      { enabled: true, cwd, command: "", args: "" },
      (p) =>
        p === join(cwd, "node_modules", ".bin", "tsx") ||
        p === join(cwd, "src", "index.ts") ||
        p === join(cwd, "agents.config.yaml"),
      join,
    );
    assert.ok(!("error" in launch));
    if ("error" in launch) return;
    assert.equal(launch.command, join(cwd, "node_modules", ".bin", "tsx"));
    assert.deepEqual(launch.args, [join(cwd, "src", "index.ts")]);
    assert.equal(launch.env.WORKSPACE_CWD, cwd);
    assert.equal(launch.env.AGENT_ORCHESTRATOR_CONFIG, join(cwd, "agents.config.yaml"));
  });

  it("does not copy Late RPC or provider keys into MCP stdio env", () => {
    const got = mcpStdioEnv(
      {
        PATH: "/usr/bin",
        HOME: "/home/operator",
        LATE_RPC_TOKEN: "rpc-token",
        LATE_DAEMON_HTTP: "http://127.0.0.1:7420",
        LATE_DAEMON_WS: "ws://127.0.0.1:7420/ws",
        CURSOR_API_KEY: "cursor-key",
        ANTHROPIC_API_KEY: "ant-key",
        OPENAI_API_KEY: "oai-key",
      },
      { WORKSPACE_CWD: MCP_CWD, LATE_RPC_TOKEN: "should-not-stick" },
    );
    assert.equal(got.PATH, "/usr/bin");
    assert.equal(got.HOME, "/home/operator");
    assert.equal(got.WORKSPACE_CWD, MCP_CWD);
    assert.equal(got.LATE_RPC_TOKEN, undefined);
    assert.equal(got.LATE_DAEMON_HTTP, undefined);
    assert.equal(got.LATE_DAEMON_WS, undefined);
    assert.equal(got.CURSOR_API_KEY, undefined);
    assert.equal(got.ANTHROPIC_API_KEY, undefined);
    assert.equal(got.OPENAI_API_KEY, undefined);
  });

  it("keeps PATH HOME USER LANG NODE_PATH so the MCP child can find node", () => {
    const got = mcpStdioEnv(
      {
        PATH: "/usr/bin:/usr/local/bin",
        HOME: "/home/operator",
        USER: "operator",
        LANG: "en_US.UTF-8",
        NODE_PATH: "/usr/lib/node_modules",
        XDG_CONFIG_HOME: "/home/operator/.config",
        CURSOR_API_KEY: "drop-me",
      },
      { WORKSPACE_CWD: MCP_CWD },
    );
    assert.equal(got.PATH, "/usr/bin:/usr/local/bin");
    assert.equal(got.HOME, "/home/operator");
    assert.equal(got.USER, "operator");
    assert.equal(got.LANG, "en_US.UTF-8");
    assert.equal(got.NODE_PATH, "/usr/lib/node_modules");
    assert.equal(got.XDG_CONFIG_HOME, "/home/operator/.config");
    assert.equal(got.CURSOR_API_KEY, undefined);
  });

  it("maps MCP schema onto OpenAI tools", () => {
    const t = mcpToOpenAiTool({
      name: "list_agents",
      description: "List specialists",
      inputSchema: { type: "object", properties: {} },
    });
    assert.equal(t.function.name, "mcp_list_agents");
    assert.match(t.function.description, /MCP/);
    assert.doesNotMatch(t.function.description, /Approve/);
    const w = mcpToOpenAiTool({ name: "start_vllm", description: "Start a local server" });
    assert.match(w.function.description, /Approve/);
  });

  it("forwards use_all_gpus onto start_vllm only", () => {
    assert.deepEqual(withStartVllmGpuArgs("mcp_start_vllm", { model_id: "qwen" }, true), {
      model_id: "qwen",
      use_all_gpus: true,
    });
    assert.deepEqual(withStartVllmGpuArgs("start_vllm", { model_id: "qwen", use_all_gpus: false }, true), {
      model_id: "qwen",
      use_all_gpus: false,
    });
    assert.deepEqual(withStartVllmGpuArgs("list_hardware", { x: 1 }, false), { x: 1 });
  });

  it("prefers HTTP URL over folder when both are set", () => {
    const cwd = MCP_CWD;
    const t = resolveMcpTarget(
      {
        enabled: true,
        cwd,
        command: "",
        args: "",
        url: "http://10.0.0.12:8790/mcp",
      },
      () => true,
      join,
    );
    assert.ok(!("error" in t));
    if ("error" in t) return;
    assert.equal(t.mode, "http");
    if (t.mode === "http") assert.equal(t.url, "http://10.0.0.12:8790/mcp");
  });

  it("uses HTTP when the folder is empty", () => {
    const t = resolveMcpTarget(
      { enabled: true, cwd: "", command: "", args: "", url: "http://192.168.1.5:8790/mcp" },
      () => false,
      join,
    );
    assert.ok(!("error" in t));
    if ("error" in t) return;
    assert.equal(t.mode, "http");
  });

  it("MCP off does not require orchestrator HTTP or a folder", () => {
    const t = resolveMcpTarget(
      {
        enabled: false,
        cwd: MCP_CWD,
        command: "",
        args: "",
        url: "http://127.0.0.1:8790/mcp",
      },
      () => true,
      join,
    );
    assert.deepEqual(t, { error: "MCP is off in Settings" });
    const launch = resolveMcpLaunch(
      { enabled: false, cwd: MCP_CWD, command: "", args: "" },
      () => true,
      join,
    );
    assert.deepEqual(launch, { error: "MCP is off in Settings" });
  });

  it("spawns the folder when the address is empty", () => {
    const cwd = MCP_CWD;
    const t = resolveMcpTarget(
      { enabled: true, cwd, command: "", args: "", url: "" },
      (p) =>
        p === join(cwd, "node_modules", ".bin", "tsx") ||
        p === join(cwd, "src", "index.ts") ||
        p === join(cwd, "agents.config.yaml"),
      join,
    );
    assert.ok(!("error" in t));
    if ("error" in t) return;
    assert.equal(t.mode, "stdio");
  });

  it("rejects file, shell, and credential URLs", () => {
    assert.ok("error" in parseMcpHttpUrl("file:///tmp/mcp"));
    assert.ok("error" in parseMcpHttpUrl("javascript:alert(1)"));
    assert.ok("error" in parseMcpHttpUrl("ftp://10.0.0.12/mcp"));
    assert.ok("error" in parseMcpHttpUrl("http://10.0.0.12:8790/mcp; rm -rf /"));
    assert.ok("error" in parseMcpHttpUrl("http://user:pass@10.0.0.12:8790/mcp"));
    const ok = parseMcpHttpUrl("http://10.0.0.12:8790/mcp");
    assert.equal(typeof ok, "string");
  });

  it("does not require Cloud AI for a private MCP host", () => {
    const parsed = parseMcpHttpUrl("http://10.0.0.12:8790/mcp");
    assert.equal(typeof parsed, "string");
    if (typeof parsed !== "string") return;
    assert.equal(classifyChatBaseSync(parsed), "private");
    assert.equal(chatEgressAllowed(classifyChatBaseSync(parsed), false), true);
    const loop = parseMcpHttpUrl("http://127.0.0.1:8790/mcp");
    assert.equal(typeof loop, "string");
    if (typeof loop !== "string") return;
    assert.equal(classifyChatBaseSync(loop), "loopback");
    assert.equal(chatEgressAllowed(classifyChatBaseSync(loop), false), true);
  });

  it("canonicalizes /MCP and accepts the GUI port when /mcp is the path", () => {
    const ok = parseMcpHttpUrl("http://127.0.0.1:8790/MCP");
    assert.equal(ok, "http://127.0.0.1:8790/mcp");
    const root = parseMcpHttpUrl("http://127.0.0.1:8790/");
    assert.equal(root, "http://127.0.0.1:8790/mcp");
    const gui = parseMcpHttpUrl("http://localhost:8787/MCP");
    assert.equal(gui, "http://127.0.0.1:8787/mcp");
    const guiRoot = parseMcpHttpUrl("http://127.0.0.1:8787/");
    assert.equal(guiRoot, "http://127.0.0.1:8787/mcp");
    const custom = parseMcpHttpUrl("http://127.0.0.1:8107/mcp");
    assert.equal(custom, "http://127.0.0.1:8107/mcp");
    assert.equal(parseMcpHttpUrl("http://localhost:8790/mcp"), "http://127.0.0.1:8790/mcp");
    assert.equal(parseMcpHttpUrl("http://[::1]:8790/mcp"), "http://127.0.0.1:8790/mcp");
  });

  it("requires Cloud AI for a public SaaS MCP host", () => {
    const parsed = parseMcpHttpUrl("https://mcp.example.com/mcp");
    assert.equal(typeof parsed, "string");
    if (typeof parsed !== "string") return;
    const egress = classifyChatBaseSync(parsed);
    assert.equal(egress, "cloud");
    assert.equal(chatEgressAllowed(egress, false), false);
    assert.equal(chatEgressAllowed(egress, true), true);
    const loop = parseMcpHttpUrl("http://127.0.0.1:8790/mcp");
    assert.equal(typeof loop, "string");
    if (typeof loop !== "string") return;
    assert.equal(chatEgressAllowed(classifyChatBaseSync(loop), false), true);
  });
});
