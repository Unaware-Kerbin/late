# Late — Local AI Terminal Emulator

Late is a desktop app for SSH, serial consoles, file transfer (SCP), REST APIs, packet capture, and **Staging** drafts (CLI, Ansible, Netmiko, Salt, Chef). Terminals can color keywords (down/up) and use a font and size you pick. An optional AI helper can propose commands; Late only sends them after you click **Approve**. Passwords and keys stay on your computer.

By default the helper uses a model **on your computer**. Cloud backends (Cursor and similar) stay off until they are enabled in Settings. Hugging Face model downloads stay available.

Late is **not** SOC 2 or ISO 27001 certified. The project’s security documents are in [docs/isms/](docs/isms/README.md). Release notes: [CHANGELOG.md](CHANGELOG.md).

## Demo

[![Late walkthrough](docs/assets/late-demo.webp)](https://github.com/Unaware-Kerbin/late/blob/main/docs/assets/late-demo.mp4)

[Full walkthrough (MP4)](https://github.com/Unaware-Kerbin/late/blob/main/docs/assets/late-demo.mp4)

## Download

Prebuilt apps are on **[Releases](https://github.com/Unaware-Kerbin/late/releases)** (currently [v0.1.3](https://github.com/Unaware-Kerbin/late/releases/tag/v0.1.3)), not on the green check next to a commit. That check is the **ci** workflow (Rust tests). Installers are built by **Build installers** and attached to the Release.

| Your computer | File on the Release | What to do |
|---|---|---|
| Windows | `Late-0.1.0-win-x64.exe` | Double-click. If SmartScreen appears: **More info** → **Run anyway**. |
| macOS (Apple Silicon) | `Late-0.1.0-mac-arm64.dmg` (or `.zip`) | Open the disk image and drag Late to Applications. First open: **right-click** Late → **Open**. |
| Linux | `Late-0.1.0-linux-x86_64.AppImage` or `Late-0.1.0-linux-amd64.deb` | AppImage: mark as executable, then double-click. Debian/Ubuntu: install the `.deb` from the download folder. |

The `0.1.0` in the filename is the app version; the GitHub tag may be newer (`v0.1.3`). Installers are unsigned. Each package includes the UI, daemon, and agent helper. SSH still uses the `ssh` program already on your computer. Ansible, Netmiko, Salt, Chef, and `sshpass` are **not** bundled — install those yourself if you want Staging **Run playbook**.

If Releases has no files yet, [install from source](#install-from-source).

## Install from source

Build Late on your computer (Windows, macOS, or Linux). Install these once, then **close and reopen** the terminal:

1. [Git](https://git-scm.com/downloads) — on Windows, use **Git Bash** for the commands below.
2. [Node.js 22 LTS](https://nodejs.org/).
3. [Rust](https://rustup.rs/) — accept the defaults. On Windows, choose the MSVC toolchain if asked.
4. Extra packages:
   - **Ubuntu / Debian:** `sudo apt install pkg-config libudev-dev build-essential openssh-client libudev1 tcpdump`
   - **macOS:** `xcode-select --install`
   - **Windows:** optional [Npcap](https://npcap.com/) or Wireshark if you want packet capture.

Then:

```bash
git clone https://github.com/Unaware-Kerbin/late.git
cd late
npm install
./late
```

`./late` starts Electron with `--no-sandbox` (needed on Linux for GPU/Wayland). Packaged AppImage/deb installers from Releases do not pass that flag. Keep the source launcher as-is unless you have a working Chromium sandbox helper.

Without Git: on the repository page choose **Code** → **Download ZIP**, unzip it, open a terminal in that folder, then run `npm install` and `./late`.

The first start compiles the backend and can take several minutes. A Late window should open. If it does not, the launcher may print a local URL — open that in a browser.

**Linux application menu:**

```bash
./late --install
```

Then search for **Late**, or run `late` in a terminal. If the command is not found, log out and back in, or add `~/.local/bin` to your PATH.

### Common problems

| What you see | What to do |
|---|---|
| `git: command not found` | Install Git and open a new terminal (Windows: Git Bash). |
| `npm: command not found` | Install Node.js 22 and open a new terminal. |
| `cargo: command not found` | Finish the rustup install and open a new terminal. |
| Compiles, but no window | On Linux/macOS check `/tmp/late-electron.log`, or open the URL printed in the terminal. |
| Serial ports or capture unavailable | Install the extras in step 4. |

## First launch

1. In the left sidebar, click **+** or right-click **Sessions** (or empty tree) to **Add folder** / **Add device**. Connect. Confirm the host key the first time you are asked.
2. The Agent pane is optional. For local AI, install [Ollama](https://ollama.com), choose **Ollama** in Late, then **Pull** a model (for example `gemma3:4b`).
3. To use Cursor or other cloud chat, open Settings and turn on the **Cloud AI** switch. Save. Session text may then leave your computer.
4. **Staging** (titlebar or Tools) holds CLI / Ansible / Netmiko / Salt / Chef drafts the helper writes on your computer. Ask for a playbook by name. **Push** is click-only (Enter does not confirm). See [Staging](#staging).
5. On an SSH device, **SCP / SFTP** copies files and folders with OpenSSH `scp`. See [File transfer](#file-transfer-scp).
6. Settings → **Keyword highlights** colors `down` / `up` (and the rest of those schemes). **Terminal font** and **size** are on the same Settings page. See [Keyword highlights](#keyword-highlights) and [Terminal look](#terminal-look).

## Help

- Questions and bugs: [GitHub Issues](https://github.com/Unaware-Kerbin/late/issues) — include your OS, Late version or commit, and what you expected vs what happened. Do not paste passwords, API keys, or `sidecar.token`.
- Security reports: [SECURITY.md](SECURITY.md) (private advisory preferred).
- Agent commands: Late waits for **Approve**. Network OS sessions also check a vendor permit list first; Linux CLI has no permit list (Approve every command). API GETs (`propose_api_get`) are operator click only (no vendor permit list). Host-key and approval dialogs are not dismissed by clicking outside. **Trust** and **Approve** need an explicit button click (not Enter). Escape does not Trust.
- Staging: see [Staging](#staging).
- File copies: see [File transfer](#file-transfer-scp). The helper cannot SCP.
- Keyword colors: Settings → **Keyword highlights**. Change the Down / Up / Warn scheme colors; they apply to SSH, serial, and local terminals. Display only.
- Terminal font and size: Settings → **Terminal font**. See [Terminal look](#terminal-look).

## Staging

The helper can draft CLI, Ansible, Netmiko, Salt, or Chef into the Staging pane (`propose_staged_artifact`). That tool only **saves a draft** on your computer. It does not log in, does not Push, and does not run playbooks.

**Push** is operator-only (click; Enter does not confirm):

| Format | Button | What happens |
|---|---|---|
| CLI | Push CLI… | Types the draft into an SSH or serial session you already opened. |
| Ansible | Run playbook… | Runs `ansible-playbook` on your computer against the **SSH inventory device** you pick (not the Push-session dropdown). |
| Netmiko | Run script… | Runs `python3` on the saved script. |
| Salt | Run state… | Runs `salt-call` / `salt-ssh`. |
| Chef | Run recipe… | Runs `chef-apply` / `chef-client`. |

Late does **not** install those PATH tools. If one is missing, you get an install error. Pick an SSH inventory host (hostname/IP + user + key, agent, or a password already saved in Late). An open serial **Push session** is only for Push CLI.

Draft files reject password-like lines and never contain `ansible_ssh_pass`. Password-only Push injects the vault secret at run time (`sshpass -e` on the daemon child) — prefer a key or agent when you can. Generic Vendor/OS does not assume Cisco IOS.

## File transfer (SCP)

On an SSH device, **SCP / SFTP** opens a dual-pane browser. Listing uses `ssh … ls`; copies use OpenSSH `scp` on your computer (folders use `scp -r`). Local paths stay under your home directory or Late data. Passwords use the same ASKPASS helper as SSH — not the process command line, and not the agent. The helper cannot upload or download files.

## Keyword highlights

Settings → **Keyword highlights** colors words in SSH, serial, and local output (SecureCRT-style). **Down / bad**, **Up / good**, and **Warn** each have a color you pick — every keyword in that scheme follows. Individual keywords can use a custom color. Optional fill paints a background. This is display-only: session logs and the agent still see the raw text. Stored on this computer, not in the daemon.

## Terminal look

Settings → **Terminal font** picks a monospace family already on your computer (system default, Cascadia, Consolas, Menlo/SF Mono, DejaVu, Ubuntu Mono, Courier, or a **Custom** name such as Fira Code). Late does not download fonts. **Size** is 10–48px (slider or the box); Ctrl+mouse wheel also zooms. UI typeface (system / accessible / all mono) is separate, under **UI typeface**.

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


