# Incident response

**Report** security issues as described in [SECURITY.md](../../SECURITY.md) at the repository root.

**Severity (project):**

- **High** — token or vault disclosure, unauthenticated RPC, Cloud AI without opt-in
- **Medium** — path jail bypass with token, supply-chain in recommended Hub ids
- **Low** — documentation mismatch, unauthenticated `/health` liveness

**Response (maintainers):** acknowledge, patch on `main`, CHANGELOG note, rotate any leaked tokens (delete `sidecar.token` and restart so a new token is created; re-save provider keys if needed).

**Evidence:** `audit.jsonl` on the affected computer; GitHub issue or private report; CI logs. There is no 24/7 on-call.

**Tabletop:** organizational; run before a Type II period.
