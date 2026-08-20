# Late — Local AI Terminal Emulator

Late is a desktop app for SSH, serial consoles, file transfer (SFTP), REST calls, and packet capture, with an optional AI helper. The helper only sends commands after you click **Approve**. Passwords and keys stay on your computer.

Chat starts **local-only**. Cloud backends (Cursor and similar) stay off until you turn them on in Settings. Hugging Face model download stays available. SOC 2 and ISO 27001 are an **aim** ([docs/isms/](docs/isms/README.md)), not a certification. Report problems in [SECURITY.md](SECURITY.md).

What changed recently: [CHANGELOG.md](CHANGELOG.md) (also [copied below](#changelog)).

## Install

You want the **download** path if a release exists. Use **from source** only if Releases is empty or you want the latest code.

### Download the app (easiest)

1. Open [github.com/Unaware-Kerbin/late/releases](https://github.com/Unaware-Kerbin/late/releases).
2. Download the file that matches your computer (newest release at the top).
3. Install or open that file:

| Your computer | File to get | What to do |
|---|---|---|
| **Windows** | installer `.exe` (or portable `.exe`) | Double-click and follow the prompts. If Windows SmartScreen says it is unrecognized, click **More info** → **Run anyway**. Installers are not code-signed yet. |
| **macOS** | `.dmg` (or `.zip`) | Open the disk image and drag Late into Applications. The first time, **right-click** Late → **Open** (it is unsigned). |
| **Linux** | `.AppImage` or `.deb` | **AppImage:** make it executable (right-click → Properties → “Allow executing file as program”, or `chmod +x` the file), then double-click it. **Debian/Ubuntu:** double-click the `.deb`. In a terminal, from the folder you downloaded to: `sudo apt install ./the-file-you-downloaded.deb` |

If that page has no files yet, skip to [Run from source](#run-from-source-linux-and-macos).

The download includes the window, the local daemon, and the agent helper. It still uses the `ssh` program already on your computer.

**Linux extras** (optional, only if you need them):

- SSH: usually already installed (`openssh-client`)
- Serial ports: `libudev`
- Packet capture: `tcpdump` (or Wireshark’s dumpcap)

On Ubuntu or Debian:

```bash
sudo apt install openssh-client libudev1 tcpdump
```

### Run from source (Linux and macOS)

This copies the project and builds it on your machine. Plan for several minutes the first time. Windows users should prefer the [download](#download-the-app-easiest).

**One-time tools** (install each, then close and reopen the terminal):

1. [Git](https://git-scm.com/downloads) — copies the project.
2. [Node.js 22 LTS](https://nodejs.org/) — pick the “LTS” installer, not an odd-numbered “Current” release if you have a choice.
3. [Rust](https://rustup.rs/) — copy the command from that page, run it, and answer the prompts with the defaults.
4. On Ubuntu or Debian, also install build packages:

```bash
sudo apt install pkg-config libudev-dev build-essential
```

**Then, once:**

```bash
git clone https://github.com/Unaware-Kerbin/late.git
cd late
npm install
./late
```

No Git? On the GitHub page click the green **Code** button → **Download ZIP**, unzip it, open a terminal in that folder, then run `npm install` and `./late`.

A Late window should open. The first run compiles the backend and can look idle for a few minutes. If Electron is missing, the script may open the UI in your browser instead.

**Put Late in the app menu (Linux):**

```bash
./late --install
```

After that you can type `late` in a terminal or find **Late** in the application menu. If `late` is “not found”, log out and back in, or add `~/.local/bin` to your PATH.

**If it fails**

| Message or symptom | Try this |
|---|---|
| `git: command not found` | Install Git, then open a **new** terminal. |
| `npm: command not found` | Install Node.js 22, then open a **new** terminal. |
| `cargo: command not found` | Finish the rustup install, then open a **new** terminal. |
| Compiles, then no window | Check `/tmp/late-electron.log`. You can still open the URL the script prints in a browser. |
| Serial or capture missing | Install the [Linux extras](#download-the-app-easiest) above. |

### After Late opens

1. In the left sidebar, add an SSH (or serial) session and connect. The first time, confirm the host key when asked.
2. The Agent pane is optional. For local AI, the simplest path is [Ollama](https://ollama.com): install it, start it, choose **Ollama** in Late, then click **Pull** and pick a model.
3. Cloud chat (Cursor and non-loopback URLs) stays off until Settings → **Allow cloud agent backends**. Session text may then leave this computer.

## For developers

### Backend (this crate)

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

`scripts/isolation-check.sh` and `cargo run -p isolation-check` grep `apps/agent-sidecar` for `keyring`, `russh`, and `secret` and fail if any hit is found. CI also runs `cargo audit` and `npm audit --omit=dev` as non-blocking advisory scans (`continue-on-error`).

## Layout

- `crates/late-core` — inventory, secrets, SSH/serial/PTY/SFTP, policy, redact, pcap, API client
- `crates/late-daemon` — Axum JSON-RPC daemon
- `crates/isolation-check` — sidecar firewall grep
- `policies/` — vendor YAML permit lists
- `apps/desktop` — Vite + React + xterm.js UI (Tauri 2 shell)
- `apps/agent-sidecar` — six-tool agent (vLLM, llama.cpp, Ollama, or Cursor SDK)
- `docker/` — optional Intel XPU vLLM example only

## Frontend + sidecar

Everyday install is in [Install](#install) (`./late` or a downloaded app). This section is the developer wiring.

Node 22 (`$HOME/.local/bin` on PATH). Workspaces: `apps/desktop`, `apps/agent-sidecar`.

```bash
npm install
npm run typecheck
npm run dev                 # daemon if built + sidecar :7430 + Vite :5173
# or separately:
npm run dev:ui
npm run dev:sidecar
```

UI talks to the daemon at `http://127.0.0.1:7420` (`GET /health`, `WS /ws` JSON-RPC). Connection errors surface in the status bar and toasts — the views are wired even if the daemon is still catching up.

Agent chat talks to the sidecar at `http://127.0.0.1:7430` (`POST /chat`, `GET /models`, `POST /approve`, `POST /stop`). local vLLM: `http://127.0.0.1:8000/v1`. llama.cpp: `http://127.0.0.1:8080/v1`. Ollama: `http://127.0.0.1:11434/v1`. Cursor: `@cursor/sdk` with `tools: []` and six `local.customTools` — only after Settings **Allow cloud agent backends**. If the SDK import fails, the sidecar returns a clear error. vLLM Start/Download stay on the local vLLM backend (Intel compose gate). llama.cpp Download/Start and Ollama Pull are in the Agent pane when those backends are selected.

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

### Build installers

GitHub Actions builds packages when you push a `v*` tag (or run **Build installers**). To pack on the machine you are sitting at:

```bash
npm install
./scripts/pack.sh          # current OS
./scripts/pack.sh --linux  # AppImage + deb
```

Output lands in `apps/desktop/release/`. Cross-building Windows/macOS from Linux is what CI is for.

### Local inference (any GPU)

Late does not bundle CUDA, ROCm, oneAPI, Ollama, llama.cpp, or model weights. The agent talks to a local vLLM (or llama.cpp / Ollama) on loopback. NVIDIA, AMD, and Intel are all fine if the server you run uses that GPU.

See `docker/README.md` for the full matrix. Short version:

| Agent pane | Default URL | Typical runtime |
|---|---|---|
| Ollama | `http://127.0.0.1:11434/v1` | Ollama (install yourself; Pull from the Agent pane) |
| llama.cpp | `http://127.0.0.1:8080/v1` | `llama-server` (CUDA, Vulkan, Metal, ROCm, or CPU) |
| local vLLM | `http://127.0.0.1:8000/v1` | vLLM on NVIDIA, AMD, or Intel |

`docker/compose.yml` is an **optional Intel XPU** vLLM example only. NVIDIA/AMD users should not start from that file.

```bash
# optional Intel Arc / XPU compose
./docker/detect-gpus.sh docker/.env
docker compose -f docker/compose.yml up
```

### Ollama (optional — easiest local AI)

Late does not install Ollama for you. On [ollama.com](https://ollama.com), download the installer for your OS, open it, and wait until Ollama is running (it stays in the background). In Late’s Agent pane choose **Ollama**, then **Pull** a name such as `gemma3:4b` or `qwen2.5:7b`. You can also Pull a Hugging Face id (`google/gemma-3-4b-it-qat-q4_0-gguf`; Late sends it as `hf.co/…`). Pull only talks to this computer. Default API: `http://127.0.0.1:11434/v1`.

### llama.cpp (optional)

Late does not install llama.cpp. Build or install [llama.cpp](https://github.com/ggml-org/llama.cpp) with the backend you want (CUDA, Vulkan, Metal, ROCm, or CPU). In the Agent pane choose **llama.cpp**, **Download** a GGUF Hugging Face id (for example `Qwen/Qwen3-8B-GGUF`, `google/gemma-3-4b-it-qat-q4_0-gguf`, or `…:Q4_K_M`). Files land in `~/.local/share/late/models/gguf/` (not the Intel vLLM Docker cache). **Start** runs `llama-server` on loopback if it is on `PATH`; otherwise run it yourself:

```bash
llama-server -m ~/.local/share/late/models/gguf/Qwen--Qwen3-8B-GGUF/*.gguf --port 8080 --host 127.0.0.1
```

Default API: `http://127.0.0.1:8080/v1`. Gated Hub repos need `HF_TOKEN` in the environment. vLLM Docker Start/Download stay hidden on this backend.

## Changelog

Dates and times are from git commits (America/New_York). Newest first. Same notes as [CHANGELOG.md](CHANGELOG.md).

### 2026-08-20 11:45

**SOC 2 / ISO 27001 ISMS (aim, not a certificate) and local-only chat by default.**

- Documented ISMS in [docs/isms/](docs/isms/README.md) (scope, policy, risk, SoA mapping Annex A to SOC 2 TSC, assets, suppliers, access, change, incident). Report issues via [SECURITY.md](SECURITY.md). Do not treat this as SOC 2 or ISO 27001 certification.
- `cloud_chat_enabled` defaults to false. Settings checkbox enables Cursor and non-loopback OpenAI-compatible chat; the toggle is audited. Hugging Face GGUF download and Ollama Pull stay available.
- `/chat` is audited (backend + loopback vs cloud, no bodies). `pcap_dir` cannot expand the path jail. Downloaded GGUF files must start with the `GGUF` magic. Google Fonts dropped from the UI CSP.

### 2026-08-20 11:13

**Recommend more SFW Hub families (Gemma, Llama, Mistral, Phi, Granite) alongside Qwen.**

- Agent pane catalogs for local vLLM, llama.cpp GGUF, and Ollama Pull include Google Gemma, Meta Llama, Mistral, Microsoft Phi, and IBM Granite instruct ids, plus Qwen. Uncensored/NSFW fine-tunes stay out of the list. Any other Hub id can still be typed. Gemma and Llama are gated — set `HF_TOKEN` after accepting the license.

### 2026-08-20 10:47

**Hugging Face download for llama.cpp, and Ollama Pull (including Hub ids).**

- llama.cpp Agent pane can Download GGUF from Hugging Face into `~/.local/share/late/models/gguf/` (no Intel Docker). Optional Start/Stop if `llama-server` is on PATH.
- Ollama Agent pane can Pull library names or Hugging Face ids (`hf.co/…`) through the local Ollama API on loopback. Late still does not install Ollama or llama.cpp.

### 2026-08-20 09:10

**Vendor-agnostic local inference, llama.cpp, and hide GPU controls when unused.**

- Docs no longer headline a specific Intel Arc card. NVIDIA, AMD, and Intel GPUs all work if the server you run uses them. `docker/compose.yml` is an optional Intel XPU vLLM example only.
- Agent pane and Settings show vLLM / GPU start-stop / Hugging Face download only when **local vLLM** is selected. Ollama, llama.cpp, and Cursor hide those personal-runtime controls. *(Superseded for llama.cpp Download and Ollama Pull by 10:47.)*
- Start / Download refuse NVIDIA and AMD (they would have pulled `intel/llm-scaler-vllm`). Use Ollama, llama.cpp, or your own CUDA/ROCm server. `LATE_VLLM_FORCE=1` overrides.
- First-class **llama.cpp** backend (`llama-server` at `http://127.0.0.1:8080/v1`). Ollama, Cursor SDK, and local vLLM remain available. Late still does not bundle CUDA, ROCm, llama.cpp, Ollama, or model weights.

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


