# Information security policy

Late is a local terminal workspace with an optional agent. Secrets and host keys stay in the Rust daemon. The UI and agent sidecar must not become a second vault.

**Principles**

1. **Loopback by default.** Daemon and sidecar bind `127.0.0.1`. Non-loopback bind requires `LATE_INSECURE_BIND=1` and is a lab exception.
2. **Token is authentication.** Browser Origin is CORS metadata only. Privileged RPC and sidecar `/chat` require the sidecar token. The UI WebSocket sends that token as the `late.<token>` subprotocol, not as a query string. Sidecar HTTP uses `X-Late-Token` or `Authorization: Bearer` only — not `?token=`.
3. **Local inference first.** Cloud agent backends (Cursor and non-loopback OpenAI-compatible URLs) are off until the operator turns on **Cloud AI** in Settings. That toggle is audited. Session text may then leave the operator’s computer.
4. **Hugging Face is for weights.** GGUF download and Ollama Pull of Hub ids are operator-initiated. They are not chat APIs.
5. **Sidecar gates.** Dual-gate is sidecar-only. On network OS, agent `propose_command` runs the vendor permit list, then the operator must Approve. Linux CLI has no permit list (Approve every command). `propose_api_get` is operator click only (no vendor permit list). Always-allow (non-Linux) still re-runs the permit check. Daemon `session.input` is ungated for a client that already has the token. The agent cannot SFTP or call `inference.download`.
6. **Least surprise.** Do not claim encryption for XOR session export. Do not claim SOC 2 or ISO certification in product copy.
7. **Need to know.** Audit logs record method, success, actor (OS user when available), and cloud vs loopback — never tokens, passwords, or command bodies.

`LATE_INSECURE_BIND=1` disables the loopback Host header check. Host is not a network control on a LAN bind.

Violations of this policy in code are defects. Organizational gaps (FDE, access reviews, auditor) remain the maintainers’ and operators’ responsibility.

Approved: 2026-08-20. Review: annual.
