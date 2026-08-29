import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { cancelAll, decide, listPending } from "./approvals.ts";
import { grantedMcpCwd, mcpChatSendArgs, setGrantedMcpCwd } from "./mcp-chat-format.ts";
import { mcpNeedsApprove } from "./mcp-format.ts";
import {
  GRANT_DIR_HOST_NOTE,
  GRANT_DIR_MISSING_NOTE,
  grantMcpAllowedDir,
  parseGrantDirPath,
} from "./mcp-grant-dir.ts";

describe("mcp grant-dir", () => {
  it("add_allowed_dir and chat_delete stay behind existing Approve", () => {
    assert.equal(mcpNeedsApprove("add_allowed_dir"), true);
    assert.equal(mcpNeedsApprove("mcp_add_allowed_dir"), true);
    assert.equal(mcpNeedsApprove("chat_delete"), true);
    assert.equal(mcpNeedsApprove("mcp_chat_delete"), true);
  });

  it("does not call add_allowed_dir until Approve, then cwd is on later chat_send", async () => {
    setGrantedMcpCwd(undefined);
    const calls: { name: string; args: Record<string, unknown> }[] = [];
    const grant = grantMcpAllowedDir("/tmp/mcp-workspace", async (name, args) => {
      calls.push({ name, args });
      return "granted";
    });
    await Promise.resolve();
    const pending = listPending();
    assert.equal(pending.length, 1);
    assert.equal(pending[0]?.title, "MCP: add_allowed_dir");
    assert.equal(pending[0]?.detail.mcp, "add_allowed_dir");
    assert.deepEqual(pending[0]?.detail.args, { path: "/tmp/mcp-workspace" });
    assert.equal(pending[0]?.allowAlwaysAllow, false);
    assert.equal(calls.length, 0);
    assert.equal(grantedMcpCwd(), undefined);
    assert.equal("cwd" in mcpChatSendArgs("before approve"), false);

    const ok = decide({ proposalId: pending[0]!.proposalId, allow: true });
    assert.equal(ok, true);
    const result = await grant;
    assert.equal(result.status, 200);
    assert.equal(result.body.ok, true);
    assert.equal(result.body.cwd, "/tmp/mcp-workspace");
    assert.equal(result.body.note, GRANT_DIR_HOST_NOTE);
    assert.deepEqual(calls, [{ name: "add_allowed_dir", args: { path: "/tmp/mcp-workspace" } }]);
    assert.equal(mcpChatSendArgs("implement README").cwd, "/tmp/mcp-workspace");
    setGrantedMcpCwd(undefined);
  });

  it("deny never calls MCP and does not set cwd", async () => {
    setGrantedMcpCwd(undefined);
    let called = 0;
    const grant = grantMcpAllowedDir("/tmp/nope", async () => {
      called += 1;
      return "should not run";
    });
    await Promise.resolve();
    const pending = listPending();
    assert.equal(pending.length, 1);
    decide({ proposalId: pending[0]!.proposalId, allow: false });
    const result = await grant;
    assert.equal(result.body.ok, false);
    assert.equal(result.body.denied, true);
    assert.equal(called, 0);
    assert.equal(grantedMcpCwd(), undefined);
    assert.equal("cwd" in mcpChatSendArgs("after deny"), false);
  });

  it("laptop-only / missing host path fails clearly and does not keep cwd", async () => {
    setGrantedMcpCwd(undefined);
    const grant = grantMcpAllowedDir("/home/laptop/only", async () => {
      throw new Error("ENOENT: no such file or directory");
    });
    await Promise.resolve();
    decide({ proposalId: listPending()[0]!.proposalId, allow: true });
    const result = await grant;
    assert.equal(result.status, 400);
    assert.equal(result.body.ok, false);
    assert.match(result.body.error ?? "", /ENOENT/);
    assert.equal(result.body.note, GRANT_DIR_MISSING_NOTE);
    assert.doesNotMatch(result.body.note ?? "", /\bSCP\b/i);
    assert.equal(grantedMcpCwd(), undefined);
  });

  it("empty path is rejected before Approve", async () => {
    const result = await grantMcpAllowedDir("  ", async () => {
      throw new Error("must not call");
    });
    assert.equal(result.status, 400);
    assert.equal(result.body.error, "path string required");
    assert.equal(listPending().length, 0);
  });

  it("cancelAll while waiting is a deny, not a tool call", async () => {
    let called = 0;
    const grant = grantMcpAllowedDir("/tmp/ws", async () => {
      called += 1;
      return "no";
    });
    await Promise.resolve();
    assert.equal(listPending().length, 1);
    cancelAll("stopped");
    const result = await grant;
    assert.equal(result.body.denied, true);
    assert.equal(called, 0);
  });

  it("non-object body and missing path are 400 before Approve", () => {
    assert.deepEqual(parseGrantDirPath(null), { ok: false, error: "JSON object with path string required" });
    assert.deepEqual(parseGrantDirPath(" /tmp "), { ok: false, error: "JSON object with path string required" });
    assert.deepEqual(parseGrantDirPath({ path: 1 }), { ok: false, error: "path string required" });
    assert.deepEqual(parseGrantDirPath({ path: "/tmp/ws" }), { ok: true, path: "/tmp/ws" });
  });

  it("Not a directory from a file drop is JSON 400 and does not throw", async () => {
    setGrantedMcpCwd(undefined);
    const grant = grantMcpAllowedDir("/tmp/notes.txt", async () => {
      throw new Error("Not a directory: /tmp/notes.txt");
    });
    await Promise.resolve();
    decide({ proposalId: listPending()[0]!.proposalId, allow: true });
    const result = await grant;
    assert.equal(result.status, 400);
    assert.equal(result.body.ok, false);
    assert.match(result.body.error ?? "", /Not a directory/);
    assert.equal(result.body.note, GRANT_DIR_MISSING_NOTE);
    assert.equal(grantedMcpCwd(), undefined);
  });

  it("callTool rejection is 400 JSON, never an unhandled rejection", async () => {
    const grant = grantMcpAllowedDir("/home/laptop/only", async () => Promise.reject("host missing"));
    await Promise.resolve();
    decide({ proposalId: listPending()[0]!.proposalId, allow: true });
    const result = await grant;
    assert.equal(result.status, 400);
    assert.equal(result.body.error, "host missing");
  });
});
