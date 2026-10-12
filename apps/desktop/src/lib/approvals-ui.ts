import type { ApprovalPrompt } from "../types";

const dismissed = new Set<string>();

export function dismissProposal(id: string) {
  if (id) dismissed.add(id);
}

export function isProposalDismissed(id: string) {
  return Boolean(id) && dismissed.has(id);
}

/** Presentation-only: what the operator is asked to Approve. Does not change gate logic. */
export function approvalPrimaryText(approval: Pick<ApprovalPrompt, "kind" | "expanded" | "detail">): string {
  const detail = approval.detail ?? {};
  if (approval.kind === "ask" && typeof detail.question === "string") {
    return detail.question;
  }
  if (typeof approval.expanded === "string" && approval.expanded.trim()) return approval.expanded;
  if (typeof detail.expanded === "string" && detail.expanded.trim()) return detail.expanded;
  if (typeof detail.command === "string" && detail.command.trim()) return detail.command;
  if (typeof detail.path === "string") {
    const method = typeof detail.method === "string" ? detail.method : "GET";
    return `${method} ${detail.path}`;
  }
  return JSON.stringify({ ...detail, expanded: approval.expanded }, null, 2);
}

export function approvalIntentLabel(detail: Record<string, unknown> | undefined): string {
  if (detail?.intent === "remediate") return "Suggested fix — changes device state";
  if (detail?.intent === "investigate") return "Read-only gather";
  return "Proposed action";
}

export function approvalSessionId(detail: Record<string, unknown> | undefined): string {
  if (!detail) return "";
  const v = detail.sessionId ?? detail.session_id;
  return typeof v === "string" && v.trim() ? v : "";
}

export function approvalProposedCommand(detail: Record<string, unknown> | undefined): string {
  if (!detail) return "";
  return typeof detail.command === "string" ? detail.command : "";
}
