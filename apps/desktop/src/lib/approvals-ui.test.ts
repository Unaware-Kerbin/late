import assert from "node:assert/strict";
import test from "node:test";
import {
  approvalIntentLabel,
  approvalPrimaryText,
  approvalProposedCommand,
  approvalSessionId,
  dismissProposal,
  isProposalDismissed,
} from "./approvals-ui.ts";

test("dismissed proposals stay dismissed", () => {
  dismissProposal("p1");
  assert.equal(isProposalDismissed("p1"), true);
  assert.equal(isProposalDismissed("p2"), false);
});

test("primary text prefers expanded command over short proposal", () => {
  assert.equal(
    approvalPrimaryText({
      kind: "command",
      expanded: "show running-config | include hostname",
      detail: { command: "show run" },
    }),
    "show running-config | include hostname",
  );
  assert.equal(
    approvalPrimaryText({
      kind: "ask",
      detail: { question: "Which VRF?" },
    }),
    "Which VRF?",
  );
});

test("intent / session helpers are presentation-only", () => {
  assert.match(approvalIntentLabel({ intent: "remediate" }), /fix/);
  assert.match(approvalIntentLabel({ intent: "investigate" }), /Read-only/);
  assert.equal(approvalSessionId({ session_id: "abc" }), "abc");
  assert.equal(approvalProposedCommand({ command: "ls" }), "ls");
});
