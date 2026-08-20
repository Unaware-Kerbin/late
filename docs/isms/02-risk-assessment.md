# Risk assessment and treatment

Likelihood and impact are qualitative (low / medium / high) for the Late **project**, not a customer’s environment. Treatment: **mitigate in product**, **accept** (documented residual), or **org** (cannot be coded).

| ID | Risk | L | I | Treatment |
|---|---|---|---|---|
| R1 | Same-OS-user process reads `sidecar.token` and calls RPC | H | H | Accept for single-user desktop. Org: lock screen, no shared logins. Mitigate: Unix 0600 token (Windows: default NTFS ACLs on the config/data dirs), loopback Host check. |
| R2 | Cloud AI sends scrollback off-box without operator intent | M | H | Mitigate: `cloud_chat_enabled` default false; sidecar refuses Cursor and non-loopback bases until opt-in; audit toggle (`settings.cloud_chat`) and sidecar `event=chat` with `backend`/`egress`/`ok` (no bodies). |
| R3 | Vaults are plaintext JSON (Unix chmod 0600 only; Windows default NTFS ACLs) | H | H | Accept this pass (no AES/age yet). Org: FDE. Later product: AEAD. |
| R4 | XOR “encrypted” export is obfuscation | M | M | Accept this pass; docs already disclose. Later: age. |
| R5 | Unsigned GGUF / Hub supply chain | M | H | Mitigate: GGUF magic `GGUF` after download; existing HF host allowlist. Residual: no SHA-256 yet. |
| R6 | `pcap_dir=/` expands path jail | L | H | Mitigate: do not add `pcap_dir` as an extra root; reject settings outside home / Late dirs. |
| R7 | Unsigned installers / `npx --yes` at pack | H | M | Accept this pass (no signing identity). Org: later pin pack tools and sign. |
| R8 | Electron `--no-sandbox` on source launchers | H | M | Accept this pass (GPU/Wayland). `./late` (Linux) and `npm run native` (every OS) pass `--no-sandbox`. Packaged AppImage/deb from electron-builder do not set the flag. Residual, not a ship gate. |
| R9 | Shared token, no unique app user | H | M | Accept for single-operator tool. Org: unique OS accounts. |
| R10 | Supplier (Cursor, HF, GitHub, npm, crates) change or incident | M | M | Org: supplier register review. Product: local default, isolation CI. |
| R11 | Audit log truncated by same user | H | M | Accept (O_APPEND, not WORM). Org: SIEM copy if required. |
| R12 | Google Fonts as extra processor | L | L | Mitigate: remove; system fonts. |

Review: 2026-08-20. Next: after material architecture change or 12 months.
