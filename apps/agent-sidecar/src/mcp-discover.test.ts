import assert from "node:assert/strict";
import { homedir } from "node:os";
import { join } from "node:path";
import { describe, it } from "node:test";
import {
  firstReachableMcpUrl,
  mcpDiscoverCandidates,
  parseAdvertisedMcpUrl,
} from "./mcp-discover.ts";

function isolated(extra?: Parameters<typeof mcpDiscoverCandidates>[0]) {
  return mcpDiscoverCandidates({
    env: {},
    homedir: "/tmp/late-discover-home",
    exists: () => false,
    readFile: () => {
      throw new Error("should not read");
    },
    ...extra,
  });
}

describe("mcp discover", () => {
  it("tries Settings URL, then GUI /mcp, then dedicated mcp:http — never inference ports", () => {
    const got = isolated({
      settingsUrl: "http://127.0.0.1:8788/mcp",
      env: { AGENT_ORCHESTRATOR_GUI_PORT: "8787", AGENT_ORCHESTRATOR_MCP_PORT: "8790" },
    });
    assert.deepEqual(got, [
      "http://127.0.0.1:8788/mcp",
      "http://127.0.0.1:8787/mcp",
      "http://127.0.0.1:8790/mcp",
    ]);
    assert.ok(!got.some((u) => /:8000|:8002/.test(u)));
    const home = homedir();
    if (home) assert.ok(!got.some((u) => u.includes(home)));
  });

  it("uses env GUI/MCP ports and still includes those URLs only on loopback", () => {
    const got = isolated({
      env: { AGENT_ORCHESTRATOR_GUI_PORT: "8107", AGENT_ORCHESTRATOR_MCP_PORT: "8798" },
    });
    assert.ok(got.includes("http://127.0.0.1:8107/mcp"));
    assert.ok(got.includes("http://127.0.0.1:8798/mcp"));
    assert.ok(!got.includes("http://127.0.0.1:8787/mcp"));
    const lan = isolated({ settingsUrl: "http://10.0.0.12:8790/mcp", env: {} });
    assert.ok(!lan.includes("http://10.0.0.12:8790/mcp"));
    assert.ok(lan.includes("http://127.0.0.1:8787/mcp"));
  });

  it("reads GUI advertise before dedicated mcp:http so last-writer mcp.url cannot hide the GUI port", () => {
    const files: Record<string, string> = {
      [join("/tmp/xdg", "agent-orchestrator", "mcp.gui.url")]: "http://127.0.0.1:8120/mcp\n",
      [join("/tmp/xdg", "agent-orchestrator", "mcp.http.url")]: "http://127.0.0.1:8790/mcp\n",
      [join("/tmp/xdg", "agent-orchestrator", "mcp.url")]: "http://127.0.0.1:8790/mcp\n",
    };
    const got = isolated({
      settingsUrl: "http://127.0.0.1:8798/mcp",
      env: { XDG_CONFIG_HOME: "/tmp/xdg" },
      exists: (p) => p in files,
      readFile: (p) => files[p] ?? "",
    });
    assert.deepEqual(got.slice(0, 4), [
      "http://127.0.0.1:8798/mcp",
      "http://127.0.0.1:8120/mcp",
      "http://127.0.0.1:8790/mcp",
      "http://127.0.0.1:8787/mcp",
    ]);
  });

  it("reads the advertised bound URL before default ports (no sibling repo path)", () => {
    const files: Record<string, string> = {
      [join("/tmp/mcp-proj", ".orchestrator", "mcp.url")]: "http://127.0.0.1:8120/mcp\n",
      [join("/tmp/xdg", "agent-orchestrator", "mcp.url")]: "http://127.0.0.1:8121/mcp\n",
      [join("/tmp/orch-state", "mcp.url")]: "http://127.0.0.1:8122/mcp\n",
    };
    const got = isolated({
      settingsUrl: "http://127.0.0.1:8798/mcp",
      mcpCwd: "/tmp/mcp-proj",
      env: {
        AGENT_ORCHESTRATOR_STATE_DIR: "/tmp/orch-state",
        XDG_CONFIG_HOME: "/tmp/xdg",
        AGENT_ORCHESTRATOR_GUI_PORT: "8787",
        AGENT_ORCHESTRATOR_MCP_PORT: "8790",
      },
      exists: (p) => p in files,
      readFile: (p) => files[p] ?? "",
    });
    assert.deepEqual(got.slice(0, 4), [
      "http://127.0.0.1:8798/mcp",
      "http://127.0.0.1:8120/mcp",
      "http://127.0.0.1:8122/mcp",
      "http://127.0.0.1:8121/mcp",
    ]);
    assert.ok(got.includes("http://127.0.0.1:8787/mcp"));
    assert.ok(got.includes("http://127.0.0.1:8790/mcp"));
  });

  it("parseAdvertisedMcpUrl keeps loopback /mcp only", () => {
    assert.equal(parseAdvertisedMcpUrl("http://127.0.0.1:8120/mcp\n"), "http://127.0.0.1:8120/mcp");
    assert.equal(parseAdvertisedMcpUrl("http://10.0.0.12:8790/mcp"), undefined);
    assert.equal(parseAdvertisedMcpUrl("not a url"), undefined);
  });

  it("firstReachable skips the dead Settings URL and returns GUI /mcp", async () => {
    const found = await firstReachableMcpUrl(
      [
        "http://127.0.0.1:8788/mcp",
        "http://127.0.0.1:8787/mcp",
        "http://127.0.0.1:8790/mcp",
      ],
      async (url) => ({ ok: url.includes(":8787/") }),
      new Set(["http://127.0.0.1:8788/mcp"]),
    );
    assert.equal(found, "http://127.0.0.1:8787/mcp");
  });
});
