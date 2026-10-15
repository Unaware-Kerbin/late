import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  MAX_SCROLLBACK_CHARS,
  MAX_TOOL_RESULT_CHARS,
  PROPOSE_COMMAND_TAIL_CHARS,
  clipToolResult,
  inferNamedStageTool,
  resolveOpenSession,
  resolveStagedFormat,
  truncateScrollbackTail,
} from "./tools.ts";

describe("stage format and session ids", () => {
  it("cli playbook infers ansible", () => {
    assert.equal(inferNamedStageTool("Write a cli playbook for this switch to configure a vlan of 2000"), "ansible");
    assert.equal(resolveStagedFormat("cli", "configure VLAN 2000", "", "Write a cli playbook"), "ansible");
  });

  it("resolves live UUID first, then nickname", () => {
    const uuid = "c10bbc8d-aaaa-bbbb-cccc-ddddeeeeffff";
    const sessions = [{ id: uuid, name: "aos-cx", kind: "ssh", vendor: "aos_cx" }];
    assert.equal(resolveOpenSession(sessions, uuid)?.id, uuid);
    assert.equal(resolveOpenSession(sessions, "aos-cx")?.id, uuid);
    assert.equal(resolveOpenSession(sessions, "missing"), undefined);
  });
});

describe("scrollback caps", () => {
  it("keeps attach and propose tails bounded", () => {
    assert.equal(MAX_SCROLLBACK_CHARS, 12_000);
    assert.equal(PROPOSE_COMMAND_TAIL_CHARS, 4_000);
    assert.ok(PROPOSE_COMMAND_TAIL_CHARS <= MAX_SCROLLBACK_CHARS);
  });

  it("truncateScrollbackTail keeps only the trailing slice", () => {
    const text = "head-" + "x".repeat(100) + "-tail";
    const out = truncateScrollbackTail(text, 20);
    assert.equal(out.length, 20);
    assert.ok(out.endsWith("-tail"));
    assert.equal(truncateScrollbackTail("short", 100), "short");
  });

  it("never returns more than maxChars even if input is multi-megabyte", () => {
    const huge = "a".repeat(2_000_000);
    assert.equal(truncateScrollbackTail(huge, MAX_SCROLLBACK_CHARS).length, MAX_SCROLLBACK_CHARS);
    assert.equal(truncateScrollbackTail(huge, PROPOSE_COMMAND_TAIL_CHARS).length, PROPOSE_COMMAND_TAIL_CHARS);
  });
});

describe("clipToolResult", () => {
  it("leaves short payloads alone", () => {
    assert.equal(clipToolResult("ok"), "ok");
  });

  it("clips multi-megabyte tool JSON under MAX_TOOL_RESULT_CHARS", () => {
    const huge = JSON.stringify({ output: "x".repeat(2_000_000) });
    const out = clipToolResult(huge);
    assert.ok(out.length <= MAX_TOOL_RESULT_CHARS + 80);
    assert.match(out, /truncated \d+ chars/);
    assert.ok(out.startsWith(huge.slice(0, 20)));
    assert.ok(out.endsWith(huge.slice(-20)));
  });
});
