# Statement of Applicability (ISO 27001:2022 Annex A)

Status: **A** = applied (product and/or this ISMS), **P** = partial, **N** = not applicable. SOC 2 column is the nearest Trust Services Criterion.

| Annex A | Title (short) | Status | SOC 2 | How Late applies |
|---|---|---|---|---|
| 5.1 | Policies | A | CC1 | This folder + README aim language |
| 5.2 | Roles | P | CC1 | Maintainers; no formal CISO role |
| 5.3 | Segregation of duties | P | CC6.1 | Sidecar agent gates vs ungated daemon `session.input`; single maintainer residual |
| 5.7 | Threat intelligence | P | CC7 | GitHub advisories; `cargo audit` / `npm audit` in CI (non-blocking first) |
| 5.10 | Acceptable use | A | CC1 | Information security policy |
| 5.19–5.23 | Suppliers | P | CC9 | Supplier register; Cloud AI opt-in |
| 5.24–5.27 | Incidents | P | CC7.3 | SECURITY.md; no 24/7 SOC |
| 5.31 | Legal | P | CC1 | Licenses in-tree; operator remains responsible for their use |
| 5.34 | Privacy | P | P | Local default; cloud opt-in; no DSR product |
| 6.1–6.6 | People | P | CC1 | Org residual (training) |
| 6.7 | Remote working | P | CC6 | Loopback runtime; org device policy |
| 7.x | Physical | N | CC6 | No Late data center |
| 8.1 | User endpoint | P | CC6 | Operator workstation |
| 8.2 | Privileged access | P | CC6 | Sidecar token; no app RBAC. Dual-gate is sidecar-only (network OS: permit list + Approve; Linux CLI: Approve every command; `propose_api_get`: click only). Daemon `session.input` is ungated for any client that already has the token |
| 8.3 | Information access | P | CC6 | Unix 0600 vaults and 0700 dirs; Windows uses default NTFS ACLs on the config/data dirs |
| 8.5 | Secure authentication | P | CC6 | Token + loopback Host (Host skipped if `LATE_INSECURE_BIND=1`); no MFA |
| 8.8 | Technical vulnerabilities | P | CC7.5 | Isolation CI + advisory scans |
| 8.9 | Configuration | P | CC8 | settings.toml; cloud flag |
| 8.15 | Logging | P | CC7 | `audit.jsonl`: daemon RPC method/ok/actor; sidecar `event=chat` with backend/egress/ok; not WORM. Clipboard IPC is not audited |
| 8.16 | Monitoring | P | CC7 | Local log only |
| 8.20–8.22 | Networks | P | CC6.6 | Loopback bind; CORS not auth |
| 8.24 | Cryptography | P | C1 | TLS/SSH in transit; at rest Unix chmod (0600/0700) or Windows default NTFS ACLs; XOR not encryption |
| 8.25–8.29 | Secure development / testing | P | CC8 | CI on PR; unsigned releases residual |
| 8.32 | Change management | P | CC8 | GitHub PRs; no CAB |
| A.5.15 / 8.10 | Deletion | P | C1.2 | auth.delete / provider clear |

**N/A with reason:** payment (no CHD), physical data center, multi-tenant hosting.

This SoA is a **project template**. A certified organization must assign owners, evidence, and review dates per control.
