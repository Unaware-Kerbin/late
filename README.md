# Late — Local AI Terminal Emulator

Linux-first terminal workspace: SSH, serial, local PTY, SFTP, REST API, and packet capture, with an investigation-only agent loop. Credentials and host keys stay in the Rust daemon; the UI and agent sidecar never hold secrets.

## Installers (macOS, Linux, Windows)

GitHub Actions builds native apps when you push a `v*` tag (or run the **Build installers** workflow):

| OS | Artifact |
|---|---|
| Linux | `.AppImage` (double-click) and `.deb` |
| macOS | `.dmg` / `.zip` (unsigned: right-click → Open the first time) |
| Windows | NSIS installer and portable `.exe` |

Each package embeds the UI, `late-daemon`, and the agent sidecar. They still use the host `ssh` client. Serial capture on Linux wants `libudev`; packet capture wants `tcpdump` (or dumpcap/Wireshark tools).

Build on the machine you are sitting at:

```bash
npm install
./scripts/pack.sh          # current OS
./scripts/pack.sh --linux  # AppImage + deb
```

Output lands in `apps/desktop/release/`. Cross-building Windows/macOS from Linux is what CI is for.

## Backend (this crate)

The daemon is a JSON-RPC service on **127.0.0.1:7420**.

```bash
# from the repo root
cargo test -p late-core
cargo build -p late-daemon
cargo run -p late-daemon
# optional: cargo run -p late-daemon -- --bind 127.0.0.1:7420
```

Requires a Rust toolchain (`rustup`), and on Linux `pkg-config` plus `libudev-dev` (for serialport). SSH sessions spawn the system `ssh` binary via a PTY (`sshpass` is used when a password is stored). Packet capture spawns host `tcpdump`.

### HTTP / WebSocket

| Endpoint | Purpose |
|---|---|
| `GET /health` | Liveness |
| `POST /rpc` | JSON-RPC (same methods as the socket) |
| `WS /ws` | JSON-RPC + `session.data` events |

CORS is limited to localhost / 127.0.0.1 / Tauri origins.

Request:

```json
{ "id": "1", "method": "inventory.list", "params": {} }
```

Success:

```json
{ "id": "1", "result": { "...": "..." } }
```

PTY output event:

```json
{ "event": "session.data", "sessionId": "...", "data": "<base64>" }
```

Methods: `inventory.list|upsert|delete`, `auth.list|upsert|delete` (passwords go to the 0600 secret file and are never returned), `settings.get|set`, `session.open` (`kind`: `ssh|serial|local|sftp|pcap|api`, `deviceId`, `acceptUnknownHost`, `cols`, `rows`, `shell`), `session.close|input|resize|reconnect|list|break|scrollback`, `policy.check`, `sftp.list|get|put|mkdir|rm`, `pcap.interfaces|start|stop|open|packets|findings|query`, `api.request`, `import.file`, `collections.list|upsert`, `capture.save|diff|export`.

Agent tools call these same session/policy/pcap/api methods. The daemon does not talk to LLMs.

Config lives under `~/.config/late/` (inventory, auth profiles, known hosts, settings, collections). Secrets are `~/.config/late/secrets.json` mode 0600. Vendor permit lists ship in `policies/*.yaml` and are copied into `~/.config/late/policies/` on first boot.

Session export with a passphrase is XOR obfuscation (`*.log.xor`), not age encryption. Encrypt the plaintext `.log` with the `age` CLI if you need real secrecy.

### Isolation CI

`scripts/isolation-check.sh` and `cargo run -p isolation-check` grep `apps/agent-sidecar` for `keyring`, `russh`, and `secret` and fail if any hit is found.

## Layout

- `crates/late-core` — inventory, secrets, SSH/serial/PTY/SFTP, policy, redact, pcap, API client
- `crates/late-daemon` — Axum JSON-RPC daemon
- `crates/isolation-check` — sidecar firewall grep
- `policies/` — vendor YAML permit lists
- `apps/desktop` — Vite + React + xterm.js UI (Tauri 2 shell)
- `apps/agent-sidecar` — six-tool agent (vLLM, Ollama, or Cursor SDK)
- `docker/` — vLLM only

## Frontend + sidecar

Node 22 (`$HOME/.local/bin` on PATH). Workspaces: `apps/desktop`, `apps/agent-sidecar`.

```bash
# One command — daemon, sidecar, native window:
./late

# Install a `late` command and application-menu entry:
./late --install
late
```

Or step by step:

```bash
export PATH="$HOME/.local/bin:$PATH"
npm install
npm run typecheck
npm run dev                 # daemon if built + sidecar :7430 + Vite :5173
# or separately:
npm run dev:ui
npm run dev:sidecar
```

UI talks to the daemon at `http://127.0.0.1:7420` (`GET /health`, `WS /ws` JSON-RPC). Connection errors surface in the status bar and toasts — the views are wired even if the daemon is still catching up.

Agent chat talks to the sidecar at `http://127.0.0.1:7430` (`POST /chat`, `GET /models`, `POST /approve`, `POST /stop`). Local vLLM: OpenAI-compatible `http://127.0.0.1:8000/v1`. Ollama: OpenAI-compatible `http://127.0.0.1:11434/v1` (not bundled). Cursor: `@cursor/sdk` with `tools: []` and six `local.customTools`. If the SDK import fails, the sidecar returns a clear error.

Safety: dual-gate command/API proposals (permit list + click). Host-key and approval modals cannot be dismissed with Enter, Escape, or click-outside. Linux sessions never get always-allow. The sidecar does not touch the keyring, russh, or secret files.

### Native window

The UI is a React app. Two native shells wrap it:

**Electron (works without extra system packages):** with the daemon, sidecar, and Vite already running:

```bash
npm run native
```

**Tauri 2** (`apps/desktop/src-tauri`) is the planned Linux-native binary. It needs GTK/WebKit headers:

```bash
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev \
  libayatana-appindicator3-dev librsvg2-dev patchelf
cd apps/desktop && npx tauri dev
```

This environment cannot install those packages (apt lock), so `npm run native` is the desktop window you can use today.

### vLLM (GPU)

See `docker/README.md`. Detect discrete B70s (skip iGPU), then:

```bash
./docker/detect-gpus.sh docker/.env
docker compose -f docker/compose.yml up
```

Default TP=2. Fallback: `TP=1 ZE_AFFINITY_MASK=1`. Image pin `intel/llm-scaler-vllm:0.21.0-b3` (fallback `0.14.0-b8.2.1` / `intel/llm-scaler-vllm`).

### Ollama (optional)

Late does not install Ollama or pull models. Install from [ollama.com](https://ollama.com), start the app, then:

```bash
ollama pull llama3.2
```

In the Agent pane choose **Ollama**. Default API: `http://127.0.0.1:11434/v1` (change under Settings if needed). If Ollama is not running, the pane shows an install/pull hint instead of silently failing.

