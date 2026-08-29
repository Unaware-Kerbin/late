import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { inferNamedStageTool, resolveOpenSession, resolveStagedFormat } from "./tools.ts";

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
