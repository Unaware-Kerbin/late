# Information security policy

Late is a local terminal workspace with an optional agent. Secrets and host keys stay in the Rust daemon. The UI and agent sidecar must not become a second vault.

**Principles**

1. **Loopback by default.** Daemon and sidecar bind `127.0.0.1`. Non-loopback bind requires `LATE_INSECURE_BIND=1` and is a lab exception.
2. **Token is authentication.** Browser Origin is CORS metadata only. Privileged RPC and sidecar `/chat` require the sidecar token.
3. **Local inference first.** Cloud agent backends (Cursor and non-loopback OpenAI-compatible URLs) are off until the operator enables **Allow cloud agent backends** in Settings. That toggle is audited. Session text may then leave the machine.
4. **Hugging Face is for weights.** GGUF download and Ollama Pull of Hub ids are operator-initiated. They are not chat APIs.
5. **Dual-gate CLI.** Agent `propose_command` runs the vendor permit list, then the operator must Approve. The agent cannot SFTP or call `inference.download`.
6. **Least surprise.** Do not claim encryption for XOR session export. Do not claim SOC 2 or ISO certification in product copy.
7. **Need to know.** Audit logs record method, success, actor (OS user when available), and cloud vs loopback — never tokens, passwords, or command bodies.

Violations of this policy in code are defects. Organizational gaps (FDE, access reviews, auditor) remain the maintainers’ and operators’ responsibility.

Approved: 2026-08-20. Review: annual.
