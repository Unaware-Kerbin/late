import { randomUUID } from "node:crypto";
import type { ApprovalDecision, ApprovalKind, PendingApproval } from "./types.js";

type Waiter = {
  pending: PendingApproval;
  resolve: (d: ApprovalDecision) => void;
  reject: (e: Error) => void;
};

const waiters = new Map<string, Waiter>();
const alwaysAllow = new Map<string, Set<string>>();

function key(kind: ApprovalKind, fingerprint: string): string {
  return `${kind}:${fingerprint}`;
}

export function isAlwaysAllowed(
  conversationId: string,
  kind: ApprovalKind,
  fingerprint: string,
  linux: boolean,
): boolean {
  if (linux) return false;
  return alwaysAllow.get(conversationId)?.has(key(kind, fingerprint)) ?? false;
}

export function rememberAlways(
  conversationId: string,
  kind: ApprovalKind,
  fingerprint: string,
  linux: boolean,
): void {
  if (linux) return;
  const set = alwaysAllow.get(conversationId) ?? new Set<string>();
  set.add(key(kind, fingerprint));
  alwaysAllow.set(conversationId, set);
}

export function requestApproval(pending: Omit<PendingApproval, "proposalId">): Promise<ApprovalDecision> {
  const proposalId = randomUUID();
  const full: PendingApproval = { ...pending, proposalId };
  return new Promise((resolve, reject) => {
    waiters.set(proposalId, { pending: full, resolve, reject });
  });
}

export function getPending(proposalId: string): PendingApproval | undefined {
  return waiters.get(proposalId)?.pending;
}

export function listPending(): PendingApproval[] {
  return [...waiters.values()].map((w) => w.pending);
}


export function decide(decision: ApprovalDecision): boolean {
  const w = waiters.get(decision.proposalId);
  if (!w) return false;
  waiters.delete(decision.proposalId);
  w.resolve(decision);
  return true;
}

export function cancelAll(reason: string): void {
  for (const [id, w] of waiters) {
    w.reject(new Error(reason));
    waiters.delete(id);
  }
}
