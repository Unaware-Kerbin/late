# Late

Late is a window on **your computer**. You use it to talk to network devices.

You can open a terminal (SSH), a serial port, a shell on this computer, copy files, look at packets, and save command drafts. A helper can *suggest* a command. Late will not send it until you click **Approve**. Passwords and keys stay on your computer.

## How it works

1. You add a device on the left (a name, an address, a login).
2. You click it. Late opens a terminal to that device.
3. You type, like any other terminal.
4. If you ask the helper, it may propose a command. You read it. You click **Approve** or **Deny**. It does not send on Enter, and it does not send by itself.
5. Cloud helpers stay **off** until you turn on **Cloud AI** in Settings. The usual helper is a model on your computer.

That is the whole idea.

## Watch it

![Watch how Late works](docs/assets/late-demo.webp)

## Install

The easy way: download a ready-made app from **[Releases](https://github.com/Unaware-Kerbin/late/releases)** (latest app tag is [v0.1.4](https://github.com/Unaware-Kerbin/late/releases/tag/v0.1.4)). Pick the file that matches your computer:

| Your computer | File | What you do |
|---|---|---|
| Windows | `Late-…-win-x64.exe` | Double-click. If Windows warns you, choose **More info** → **Run anyway**. |
| Mac (Apple chip, macOS 13+) | `Late-…-mac-arm64.dmg` | Open it. Drag Late into Applications. First time: **right-click** Late → **Open**. |
| Linux | `Late-…AppImage` or `Late-….deb` | AppImage: make it executable, then double-click. Ubuntu/Debian: install the `.deb`. |

The number in the file name is the app version. It can be older than the GitHub tag. These files are unsigned. SSH still uses the `ssh` program already on your computer.

If Releases has no app files yet, [build it yourself](#install-from-source).

## Install from source

Do this if you want to run Late from this folder. Install these **once**, then close the terminal and open a new one:

1. [Git](https://git-scm.com/downloads) — on Windows, use **Git Bash**.
2. [Node.js 22](https://nodejs.org/).
3. [Rust](https://rustup.rs/) — tap through the defaults.
4. Extra bits:
   - **Ubuntu / Debian:** `sudo apt install pkg-config libudev-dev build-essential openssh-client libudev1 tcpdump`
   - **Mac:** `xcode-select --install`
   - **Windows:** optional [Npcap](https://npcap.com/) or Wireshark if you want packet capture.

Then:

```bash
git clone https://github.com/Unaware-Kerbin/late.git
cd late
npm install
./late
```

The first start builds the backend. Wait. A Late window should open. If it does not, the terminal may print a local web address — open that.

No Git? On GitHub click **Code** → **Download ZIP**, unzip it, open a terminal in that folder, then `npm install` and `./late`.

On Linux, `./late --install` puts Late in the app menu. After that you can search for **Late**, or type `late`. If `late` is not found, log out and back in.

| Problem | Fix |
|---|---|
| `git: command not found` | Install Git. Open a new terminal. |
| `npm: command not found` | Install Node.js 22. Open a new terminal. |
| `cargo: command not found` | Finish the Rust install. Open a new terminal. |
| It builds, but no window | On Linux/macOS look at `/tmp/late-electron.log`, or open the address printed in the terminal. |

## After it opens

1. On the left, click **+** (or right-click **Sessions**) → **Add device**. Fill it in. Connect. The first time, click **Trust** for the host key (not Enter).
2. Helper is optional. Easiest local AI: install [Ollama](https://ollama.com), pick **Ollama** in Late, **Pull** a model (try `gemma3:4b`).
3. Want Cursor or other cloud chat? Settings → **Cloud AI** → Save. Then session text may leave your computer.
4. **Staging** (Tools) is a scratch pad for CLI / Ansible drafts. **Push** is a click. The helper cannot Push.
5. On an SSH device, **SCP / SFTP** copies files. The helper cannot copy files.
6. Settings has **Keyword highlights** (`down` / `up` colors) and **Terminal font** / size.

## Help

- Stuck or a bug: [GitHub Issues](https://github.com/Unaware-Kerbin/late/issues). Say your OS and what you expected. Do not paste passwords, keys, or `sidecar.token`.
- Security: [SECURITY.md](SECURITY.md).
- Late is **not** SOC 2 or ISO 27001 certified. Project notes: [docs/isms/](docs/isms/README.md). Changes: [CHANGELOG.md](CHANGELOG.md).

## For developers

### Daemon

The daemon is a JSON-RPC service on **127.0.0.1:7420**.

```bash
# from the repo root
cargo test -p late-core
cargo build -p late-daemon
cargo run -p late-daemon
# optional: cargo run -p late-daemon -- --bind 127.0.0.1:7420
```

Requires a Rust toolchain (`rustup`), and on Linux `pkg-config` plus `libudev-dev` (for serialport). SSH sessions spawn the system `ssh` binary via a PTY (passwords use `SSH_ASKPASS`, not the process environment). Packet capture spawns host `tcpdump`.

### HTTP / WebSocket

| Endpoint | Purpose |
|---|---|
| `GET /health` | Liveness |
| `POST /rpc` | JSON-RPC (same methods as the socket) |
| `WS /ws` | JSON-RPC + `session.data` events |

CORS is limited to the UI loopback origins (`127.0.0.1`/`localhost` on ports 5173, 4173, and 1420, plus Tauri). `/rpc` and `/ws` require the sidecar token (`X-Late-Token`, `Authorization: Bearer`, or the WebSocket subprotocol `late.<token>` — not a query string). The Host header must be loopback unless `LATE_INSECURE_BIND=1`. Daemon `GET /health` stays unauthenticated and returns `{"ok":true}` only.

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

Methods: `inventory.list|upsert|delete`, `auth.list|upsert|delete` (passwords go to the secret file — Unix mode 0600; Windows uses default NTFS ACLs on the config/data dirs — and are never returned), `settings.get|set`, `session.open` (`kind`: `ssh|serial|local|sftp|pcap|api`, `deviceId`, `acceptUnknownHost`, `cols`, `rows`, `shell`; `scp` is an alias for `sftp`), `session.close|input|resize|reconnect|list|break|scrollback`, `policy.check`, `sftp.list|get|put|mkdir|rm` (aliases `scp.list|get|put|upload|download`; `recursive` copies folders with `scp -r`), `pcap.interfaces|start|stop|open|packets|findings|query`, `api.request`, `import.file`, `collections.list|upsert`, `capture.save|diff|export`, `stage.render|save|get|list|plan|push`.

Agent tools call these same session/policy/pcap/api methods. The daemon does not talk to LLMs.

Config lives in the platform config directory named `late`: Linux `~/.config/late/`, macOS `~/Library/Application Support/late/`, Windows `%APPDATA%\late` (inventory, auth profiles, known hosts, settings, collections, `sidecar.token`, `secrets.json`). On Unix, secrets and tokens are mode 0600 and config/data dirs are 0700; Windows uses default NTFS ACLs on the config/data dirs. Session logs, captures, pcap, and `audit.jsonl` live in the platform data directory: Linux `~/.local/share/late/`, macOS `~/Library/Application Support/late/`, Windows `%APPDATA%\late`. Vendor permit lists ship in `policies/*.yaml` and are copied into the config `policies/` folder on first boot.

Session export with a passphrase is XOR obfuscation (`*.log.xor`), not age encryption. Encrypt the plaintext `.log` with the `age` CLI if you need real secrecy.

### Isolation CI

The **ci** workflow on each push is Rust tests, isolation greps, and advisory scans. It does **not** attach `.exe` / `.dmg` / AppImage files. Those come from **Build installers** on a `v*` tag and show up under [Releases](https://github.com/Unaware-Kerbin/late/releases).

`scripts/isolation-check.sh` and `cargo run -p isolation-check` grep `apps/agent-sidecar` so it must not contain `keyring`, `russh`, or `secrets.json` vault code, and must not call `stage.push` / `stage.plan` or file-transfer RPCs (`sftp.upload`, `scp.download`, and similar). That grep does not mean the sidecar never reads `sidecar.token` — it does, for auth. CI also runs `cargo audit` and `npm audit --omit=dev` as non-blocking advisory scans (`continue-on-error`).

## Layout

- `crates/late-core` — inventory, secrets, SSH/serial/PTY/SCP, policy, redact, pcap, API client, staging
- `crates/late-daemon` — Axum JSON-RPC daemon
- `crates/isolation-check` — sidecar firewall grep
- `policies/` — vendor YAML permit lists
- `apps/desktop` — Vite + React + xterm.js UI (Electron is the current shell; Tauri 2 is optional)
- `apps/agent-sidecar` — seven-tool agent (vLLM, llama.cpp, Ollama, or Cursor SDK), including `propose_staged_artifact`
- `docker/` — optional Intel XPU vLLM example only

## Frontend + sidecar

See [Install from source](#install-from-source) to run the app. Development layout:

Node 22. Workspaces: `apps/desktop`, `apps/agent-sidecar`.

```bash
npm install
npm run typecheck
npm run dev                 # daemon if built + sidecar :7430 + Vite :5173
# or separately:
npm run dev:ui
npm run dev:sidecar
```

UI talks to the daemon at `http://127.0.0.1:7420` (`GET /health`, `WS /ws` JSON-RPC). Connection errors surface in the status bar and toasts — the views are wired even if the daemon is still catching up.

Agent chat talks to the sidecar at `http://127.0.0.1:7430` (`GET /health` returns `{"ok":true}` only — it does not include a daemon boolean; `POST /chat`, `GET /models`, `GET /pending`, `POST /approve`, `POST /stop`). Privileged sidecar routes take the token from `X-Late-Token` or `Authorization: Bearer` (not a `?token=` query string). local vLLM: `http://127.0.0.1:8000/v1`. llama.cpp: `http://127.0.0.1:8080/v1`. Ollama: `http://127.0.0.1:11434/v1`. Cursor: `@cursor/sdk` with `tools: ["mcp"]` and seven `local.customTools` (including `propose_staged_artifact`) — only after Settings **Cloud AI** is on. If the SDK import fails, the sidecar returns a clear error. vLLM Start/Download stay on the local vLLM backend (Intel compose gate). llama.cpp Download/Start and Ollama Pull are in the Agent pane when those backends are selected.

Safety: dual-gate is sidecar-only. On network OS CLI, `propose_command` runs the vendor permit list, then you click **Approve**. Linux CLI has no permit list (Approve every command). `propose_api_get` is operator click only (no vendor permit list). Always-allow (non-Linux) still re-runs the permit check. Daemon `session.input` is ungated for a client that already has the token. Operator **Push** from Staging is UI-only (`stage.plan` / `stage.push`): CLI types into an open SSH/serial session; Ansible / Netmiko / Salt / Chef run PATH tools on your computer (Late does not bundle them). PATH Push uses the inventory device’s auth profile (key, agent, or a password already in the daemon vault — injected at run time, never written into staging files). The helper cannot Push. Host-key and approval dialogs are not dismissed by clicking outside. **Trust**, **Approve**, and Staging **Push** need an explicit button click (not Enter). Escape does not Trust. The sidecar reads `sidecar.token` for auth; it must not contain keyring, russh, or `secrets.json` vault code.

When the operator asks a specific question, the agent proposes the show/display/GET needed to answer it. After **Approve**, it sends the command, reads the device output, and continues. If it finds a problem, it also proposes the specific command to implement the fix (still gated the same way).

### Native window

The UI is a React app. Two native shells wrap it:

**Electron (works without extra system packages):** with the daemon, sidecar, and Vite already running:

```bash
npm run native
```

`npm run native` also passes `--no-sandbox` on every OS. Use it only with daemon, sidecar, and Vite already running. Packaged Release binaries do not set this flag.

**Tauri 2** (`apps/desktop/src-tauri`) is the planned Linux-native binary. It needs GTK/WebKit headers:

```bash
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev \
  libayatana-appindicator3-dev librsvg2-dev patchelf
cd apps/desktop && npx tauri dev
```

### Build installers

GitHub Actions packages Linux, macOS, and Windows when a `v*` tag is pushed (workflow **Build installers**). Finished files are on the Release for that tag, not in the **ci** job log. Pack each OS on a computer running that OS:

```bash
npm install
./scripts/pack.sh
```

Artifacts land in `apps/desktop/release/`.

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

Late does not install Ollama. On [ollama.com](https://ollama.com), download the installer for your OS, open it, and wait until Ollama is running (it stays in the background). In Late’s Agent pane choose **Ollama**, then **Pull** a name such as `gemma3:4b` or `qwen2.5:7b`. A Hugging Face id also works (`google/gemma-3-4b-it-qat-q4_0-gguf`; Late sends it as `hf.co/…`). Pull only talks to the local Ollama server. Default API: `http://127.0.0.1:11434/v1`.

### llama.cpp (optional)

Late does not install llama.cpp. Build or install [llama.cpp](https://github.com/ggml-org/llama.cpp) with the backend you want (CUDA, Vulkan, Metal, ROCm, or CPU). In the Agent pane choose **llama.cpp**, **Download** a GGUF Hugging Face id (for example `Qwen/Qwen3-8B-GGUF`, `google/gemma-3-4b-it-qat-q4_0-gguf`, or `…:Q4_K_M`). Files land in `~/.local/share/late/models/gguf/` (not the Intel vLLM Docker cache). **Start** runs `llama-server` on loopback if it is on `PATH`; otherwise run it yourself:

```bash
llama-server -m ~/.local/share/late/models/gguf/Qwen--Qwen3-8B-GGUF/*.gguf --port 8080 --host 127.0.0.1
```

Default API: `http://127.0.0.1:8080/v1`. Gated Hub repos need `HF_TOKEN` in the environment. vLLM Docker Start/Download stay hidden when llama.cpp is selected.
