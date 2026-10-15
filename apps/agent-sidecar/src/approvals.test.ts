import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  cancelAll,
  cancelConversation,
  decide,
  listPending,
  requestApproval,
} from "./approvals.ts";

describe("approvals conversation scope", () => {
  it("cancelConversation only rejects waiters for that conversationId", async () => {
    const a = requestApproval(
      { kind: "command", title: "a", detail: { command: "show a" } },
      "conv-a",
    );
    const b = requestApproval(
      { kind: "command", title: "b", detail: { command: "show b" } },
      "conv-b",
    );
    assert.equal(listPending().length, 2);

    const aRejected = a.promise.then(
      () => "resolved",
      (e: Error) => e.message,
    );
    cancelConversation("conv-a", "superseded by a new chat turn");
    assert.equal(await aRejected, "superseded by a new chat turn");
    assert.equal(listPending().length, 1);
    assert.equal(listPending()[0]?.proposalId, b.proposal.proposalId);

    assert.equal(decide({ proposalId: b.proposal.proposalId, allow: true }), true);
    assert.deepEqual(await b.promise, {
      proposalId: b.proposal.proposalId,
      allow: true,
    });
    assert.equal(listPending().length, 0);
  });

  it("cancelAll still clears every waiter", async () => {
    const a = requestApproval(
      { kind: "ask", title: "q", detail: { question: "x" } },
      "c1",
    );
    const b = requestApproval(
      { kind: "ask", title: "q2", detail: { question: "y" } },
      "c2",
    );
    const catchMsg = (p: Promise<unknown>) =>
      p.then(
        () => "resolved",
        (e: Error) => e.message,
      );
    const rejected = Promise.all([catchMsg(a.promise), catchMsg(b.promise)]);
    cancelAll("stopped");
    assert.deepEqual(await rejected, ["stopped", "stopped"]);
    assert.equal(listPending().length, 0);
  });
});
