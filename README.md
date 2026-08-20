# Late — Local AI Terminal Emulator

Linux-first terminal workspace: SSH, serial, local PTY, SFTP, REST API, and packet capture, with an investigation-only agent loop. Credentials and host keys stay in the Rust daemon; the UI and agent sidecar never hold secrets.

What landed and when is in [CHANGELOG.md](CHANGELOG.md) and is copied at the [bottom of this page](#changelog).

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

CORS is limited to the UI loopback origins (`127.0.0.1`/`localhost` on ports 5173, 4173, and 1420, plus Tauri). `/rpc` and `/ws` require the sidecar token (`X-Late-Token` or `?token=` on the WebSocket). The Host header must be loopback. `GET /health` stays unauthenticated.

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

When you ask a specific question, the agent proposes the show/display/GET needed to answer it. After you Approve, it sends the command, reads the device output, and continues. If it finds a problem, it also proposes the specific command to implement the fix (still dual-gated).

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

## Changelog

Dates and times are from git commits (America/New_York). Newest first. Same notes as [CHANGELOG.md](CHANGELOG.md).

### 2026-08-20 08:59

**Compact session tree, loopback auth, and ACL permit lists.**

Inventory

- Session manager is a one-line tree under **Sessions** (folder / host name), with a toolbar (connect, new folder, new session, properties, delete, expand/collapse).
- Filter matches name, host, folder, or tag (`Alt+I`). Arrow keys move; Enter connects; double-click connects.
- Local PTY, packet capture, and Edgeshark sit in a **Tools** group instead of crowding the device list.
- Vendor / OS is on the session form (not only under extra options).

Agent and permit lists

- Open SSH/serial sessions no longer default to a deny-all Generic list. Generic allows `show` / `ping` / `traceroute` until the MOTD or `show version` identifies the OS (Aruba AOS-CX, Cisco, Arista, and others).
- AOS-CX and Cisco allowlists include config needed for ACLs (`configure`, `access-list` / `acl`, `apply`, numbered ACEs like `10 deny tcp any any eq 80`). Destructive verbs (`start-shell`, `copy`, `reload`, `write erase`) stay denied. Config lines cannot be always-allowed.
- If the permit list still blocks a line, Late will not send it, but the agent is told to print the full CLI for you to paste.

Security

- Daemon `/rpc` and `/ws` and sidecar `/chat` `/approve` `/pending` `/models` `/stop` require the sidecar token. Origin is CORS only (it is not authentication).
- Host header must be loopback. Non-loopback `--bind` is refused unless `LATE_INSECURE_BIND=1`.
- SSH uses `StrictHostKeyChecking=yes` against a pinned OpenSSH known_hosts file after Late’s TOFU pin.
- Secrets and tokens are written mode `0600` atomically; config directories are `0700`.
- Electron stays on the app origin for navigation and `late:token` IPC. Desktop CSP limits `connect-src` to loopback.
- Path jail under home / Late data; local PTY is bash/zsh/fish/sh only; remote tcpdump BPF is quoted.
- Append-only `audit.jsonl` for privileged RPC and Approve/Deny. API TLS is verified by default.
- `./late` always restarts the sidecar on `:7430` with the token (fixes chat dying against a leftover sidecar).

### 2026-08-19 16:23

**Stop Cursor hangs after a command, and let Settings scroll.**

- Cursor idle timer no longer treats pre-tool “I’ll check…” text as the answer, and no longer closes the stream while Approve is pending.
- Stream iterator is no longer polled twice at once (that deadlock froze the agent).
- After a silent stall, one hop writes the answer from device output.
- Vendor permit check is fail-closed (`allowed === true` required).
- Settings (and other tall dialogs) scroll inside the viewport.

### 2026-08-19 14:57 — `3bfa1c4`

**Add nested inventory folders so devices can be grouped by site.**

- Inventory sidebar is a collapsible tree (SecureCRT-style). Paths like `NYC/Core` nest by `/`.
- Create, rename, and delete folders from an in-app form (Electron has no `window.prompt`).
- Drag devices onto folders or Ungrouped. New SSH/serial/API sessions inherit the selected folder.
- Empty folders stay in the tree for site planning. Rename refuses collisions and moving a folder into itself.
- Cursor agent stops looping after it has an answer instead of proposing extra `show` commands.
- Local PTY, Packet capture, and Edgeshark sit idle on the sidebar background; hover/focus gets the light grey highlight.

### 2026-08-19 12:10 — `f8e8aa6`

**Keep the agent going after an approved command and hide vLLM when using Cursor.**

- After you Approve a command, the agent reads device output and continues the answer (still dual-gated).
- When the chat backend is Cursor SDK, the local vLLM model list is hidden.

### 2026-08-19 11:34 — `aa132e2`

**Add optional Ollama backend without bundling GPU runtimes.**

- First-class Ollama option in the agent pane (`http://127.0.0.1:11434/v1`).
- Installers still do not ship vLLM, llama.cpp, CUDA, or Ollama itself.

### 2026-08-19 11:12 — `c505bef`

**Add cross-platform Late installers and GitHub Actions.**

- Electron packages the UI, `late-daemon`, and agent sidecar.
- CI builds Linux (AppImage/deb), macOS, and Windows artifacts on `v*` tags.


