# Access control

**Identities.** Late has no application user directory. The operator is the OS user who can read `sidecar.token` (Unix mode 0600; Windows uses default NTFS ACLs on the config/data dirs) in the platform config directory named `late`.

**Authentication.** Privileged daemon `/rpc` and `/ws` and sidecar `/chat`, `/approve`, `/pending`, `/models`, `/stop` require the token. The UI WebSocket presents the token as the `late.<token>` subprotocol, not as a query string. Sidecar HTTP uses `X-Late-Token` or `Authorization: Bearer` only — not `?token=`. Host header must be loopback unless `LATE_INSECURE_BIND=1`. Daemon and sidecar `/health` are unauthenticated and return `{"ok":true}` only (sidecar does not include a daemon boolean). Dev Vite serves `/__late_token` to the UI origin only.

**IPC.** Electron `late:token` / clipboard channels require a live `senderFrame` URL equal to the packaged `dist/index.html` (or the Vite origin in unpackaged). Missing frames are denied. Clipboard write and read are capped at 2 MB. Clipboard is not audited.

**Authorization.** Agent tools are six functions. Dual-gate is sidecar-only: network OS CLI is vendor permit list then Approve; Linux CLI has no permit list (Approve every command); `propose_api_get` is operator click only (no vendor permit list). Always-allow (non-Linux) still re-runs the permit check. Daemon `session.input` is ungated for a client that already has the token.

**Cloud.** Cursor and public-internet OpenAI-compatible chat require `cloud_chat_enabled` (**Cloud AI** in Settings). Private-network OpenAI-compatible URLs (RFC1918, `.internal`, `private_inference_hosts`) do not. Provider keys are write-only from the UI; the sidecar materializes them over a Origin-rejected route.

**Joiner / leaver.** Organizational: OS account lifecycle. Delete the platform config and data directories named `late` (Linux `~/.config/late` and `~/.local/share/late`; macOS/Windows `~/Library/Application Support/late` or `%APPDATA%\late`) on the operator’s computer.

**Privileged exceptions.** `LATE_INSECURE_BIND=1`, `LATE_VLLM_FORCE=1`, `api_insecure_tls`. Lab only; auditors will sample who may set them. Settings `pcap_dir` / `log_dir` are confined on save and are not extra jail roots; captures write under Late data.
