const dismissed = new Set<string>();

export function dismissProposal(id: string) {
  if (id) dismissed.add(id);
}

export function isProposalDismissed(id: string) {
  return Boolean(id) && dismissed.has(id);
}
