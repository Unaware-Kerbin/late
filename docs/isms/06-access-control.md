# Access control

**Identities.** Late has no application user directory. The operator is the OS user who can read `~/.config/late/sidecar.token` (mode 0600).

**Authentication.** Privileged daemon `/rpc` and `/ws` and sidecar `/chat`, `/approve`, `/models`, `/stop` require the token. Host header must be loopback. `/health` is unauthenticated (liveness only).

**Authorization.** Agent tools are six functions. CLI to devices is dual-gated in the sidecar (permit list then Approve). Human typed input and collections use `session.input` without the permit list.

**Cloud.** Cursor and non-loopback OpenAI-compatible chat require `cloud_chat_enabled`. Provider keys are write-only from the UI; the sidecar materializes them over a Origin-rejected route.

**Joiner / leaver.** Organizational: OS account lifecycle. Deleting `~/.config/late` and `~/.local/share/late` removes Late state on that machine.

**Privileged exceptions.** `LATE_INSECURE_BIND=1`, `LATE_VLLM_FORCE=1`, `api_insecure_tls`. Lab only; auditors will sample who may set them.
