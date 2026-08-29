# Changelog

Dates and times are from git commits (America/New_York). Newest first.

## 2026-08-29

**v0.1.7 installers.**

- GitHub Release [v0.1.7](https://github.com/Unaware-Kerbin/late/releases/tag/v0.1.7) packs Linux, macOS, and Windows from this `main` (Agent=MCP uses the advertised GUI `/mcp` URL, not Late vLLM; README points at [Agent Orchestrator](https://github.com/Unaware-Kerbin/agent-orchestrator)). File names are `Late-0.1.7-…`. Still unsigned.

## 2026-08-28

**MCP is an Agent backend, like vLLM.**

- Pick **MCP** in the first Agent menu (vLLM / llama.cpp / Ollama / MCP / …). Chat goes through that program (`chat_send`), not local vLLM. Second control is This computer (folder) or an HTTP address. Paste the printed `/mcp` URL (the GUI also serves `/mcp` on the same port as the web UI; the page itself is not MCP). If Settings still has a stale port, Late tries the URL that program advertised when it bound. Example dedicated URL: `http://127.0.0.1:8790/mcp`. Late still Approve for device CLI. Extra `chat_send` / `dispatch` / `start_vllm` still wait for Approve; Send is the one chat round. Cursor `mcpServers` stays empty. Late does not copy API keys into that program. Late still chats when MCP is off.

**Agent pane catalogs list every snapshot, with fit labeled.**

- vLLM, llama.cpp (GGUF), and Ollama each show the full catalog for that engine (not a 1–2 model slice). Newest instruct ids are marked (Qwen3 / Qwen3.8 over Qwen2.5, Gemma 4 over Gemma 3/2, Llama 4 Scout / 3.3 over 3.1). Too-big and previous-generation rows stay listed. vLLM ids are Hugging Face weights; llama.cpp ids are GGUF; Ollama ids are library tags.

**Use all GPUs on this computer.**

- Local Start (Intel vLLM Docker and llama.cpp) uses every GPU it can see on this computer by default: vLLM `--tensor-parallel-size`, llama.cpp `-ngl 99 -sm layer`. Uncheck **Use all GPUs on this computer** to stay on one card. Ollama already uses every GPU; Late does not pin it. MCP `start_vllm` does the same when Settings MCP is on. This is several cards in one box, not a cluster.

**Agent pane MCP picker (this computer / HTTP) when the backend is MCP.**

- With **MCP** selected: second control is **This computer** (stdio folder from Settings) or **HTTP address** (paste the printed `/mcp` URL; example `http://127.0.0.1:8790/mcp` after `npm run mcp:http`). The GUI root is HTML, not MCP; `/mcp` on that same port is Streamable HTTP. Status shows MCP on · N tools or MCP error. The third menu is **orchestrator** only (auto-route via `chat_send`). Specialists and vLLM / Ollama / Cloud stay in the first Agent menu. Cursor `mcpServers` stays empty.

**Optional MCP tools — folder on this computer, or Streamable HTTP on :8790.**

- Settings **MCP** (off by default) can point at a project folder on this computer (stdio) **or** a Streamable HTTP URL. Paste the printed `/mcp` URL (GUI `/mcp` on the web UI port, or `http://127.0.0.1:8790/mcp` after `npm run mcp:http`). If both folder and address are set, Late uses the address. The sidecar is the client and adds those tools as `mcp_*`. List/status tools run immediately. Starts, downloads, dispatch, and writes still wait for Approve. A homelab MCP URL does not need Cloud AI; a public internet host does. Cursor Cloud AI still uses empty `mcpServers` so Approve is not bypassed. Late does not copy API keys into that program, and does not start MCP on another machine.

## 2026-08-26

**A helper on this computer or on your network, without Cloud AI.**

- **Cloud AI** is only Cursor and the public internet. A GPU at home or in the lab is not Cloud AI.
- Agent pane: pick vLLM / llama.cpp / Ollama, then **Local** (this computer) or **Add server** (one extra address you already started). The saved host shows up in that menu. Late does not SSH-start vLLM on the other machine. Start / Pull / Download stay on this computer and hide when you are not on Local.
- Settings **Check** and the Add server dialog probe `GET /v1/models`. Optional **Private inference hosts** is for names that do not look like a home IP. Chat still needs OpenAI-style `/v1` (or LiteLLM in front).

**Anthropic, Gemini, and Azure helpers.**

- Native adapters: Anthropic Messages, Gemini generateContent, Azure OpenAI deployments (`api-key`). Same private-network gate as vLLM. Public SaaS hosts still need Cloud AI. Tool calls still drive Approve. Keys: Anthropic, Gemini, Azure.

**v0.1.6 installers.**

- GitHub Release [v0.1.6](https://github.com/Unaware-Kerbin/late/releases/tag/v0.1.6) packs Linux, macOS, and Windows from this `main` (homelab helper, **Local** / **Add server**, Anthropic / Gemini / Azure). File names are `Late-0.1.6-…`. Still unsigned.

**v0.1.5 installers.**

- GitHub Release [v0.1.5](https://github.com/Unaware-Kerbin/late/releases/tag/v0.1.5) rebuilds Linux, macOS, and Windows apps from this `main` (Electron 44, session tabs/splits, Staging editor, SCP, `undici` pin). File names are `Late-0.1.5-…`. Still unsigned. macOS pack looks for `Electron.app` (Electron 42+ no longer unzips `dist/electron` on Darwin).

**Staging editor (syntax, suggestions, secret-line marks).**

- Staging drafts use a VS Code-style editor instead of a plain box. Format picks the language (YAML for Ansible/Salt, Python for Netmiko, Ruby for Chef, plain for CLI). Ctrl+Space inserts the right skeleton — no password fields. Lines that look like credentials get a red mark before Save. Theme and terminal font follow Settings. Monaco workers stay same-origin / blob only.

**Patch `undici` 5.x from optional Cloud AI.**

- `@cursor/sdk` pulled `undici` 5.29.0 through `@connectrpc/connect-node` (5.x is EOL and has the 12 Dependabot GHSAs). The sidecar pins `undici` 6.28.0 and the workspace overrides the same version so Cloud AI does not keep 5.x. SSH, serial, and SCP do not use this package.

**Electron 44, session tabs, and splits.**

- Desktop runtime is Electron **44.0.0** (was 37.10.3; Chromium 152, Node 24 inside Electron). Packaged Mac apps need **macOS 13+**. Host Node for Vite/sidecar stays 22.
- `scripts/ensure-electron.sh` unpacks the Electron binary after Electron 42+ dropped postinstall unzip, so `./late` and GitHub packers get `node_modules/electron/dist`. Source `./late` still uses `--no-sandbox` and software GL / X11 on Linux; packaged AppImage and `.deb` do not. It no longer falls back to a browser if Electron is missing.
- Chromium 152 asks for **local-network-access** before the UI WebSocket to the loopback daemon. That permission is allowed so saved sessions load; other webview permissions stay denied. Folder-pane right-click **Add folder** / **Add device** stays open (the right-click’s follow-up pointer event no longer closes it).
- Connecting another device opens a **new tab** (an empty pane in the current tab is still reused). Live session tabs have a **border** and the device accent stripe so they are easy to find.
- **Split** left / right / up / down tiles another connected tab into this one (or adds an empty pane if you only have one session). Titlebar **◂ ▸ ▴ ▾**, tab right-click, device **Connect split …**, pane arrows, palette, or **Ctrl+Shift+arrows**.

## 2026-08-25

**SCP copies, keyword colors, and adjustable terminal fonts.**

- Removed **Edgeshark** (compose file, Tools entry, and `pcap.edgeshark.*` RPCs). Stop targeted compose project `late-edgeshark` while Ghostwire kept running as `edgeshark`, and `docker compose down` held the daemon’s blocking RPC pool so **Open in Wireshark** timed out. Packet capture, dumpcap/tcpdump, and Wireshark stay.
- File transfer is OpenSSH **scp** (folders use `scp -r`). The sidebar button is **SCP / SFTP**. RPC aliases `scp.upload` / `scp.download`. Local paths stay under home or Late data. The helper still cannot copy files.
- Terminal **keyword highlights** (Settings): `down` / `up` / warn color schemes are adjustable (not a single baked-in red/green). Display only — logs and the agent still see the raw text.
- Terminal **font and size** are adjustable in Settings (system stacks plus a custom family already on this computer; 10–48px). Ctrl+wheel still zooms size.
- README leads with how Late works and how to install, in plain language. The walkthrough is the animated `docs/assets/late-demo.webp` on the README (GitHub will not play a repo MP4 on that page). Staging, Cloud AI, PATH Push, SCP, keyword colors, and terminal fonts stay in the first-run list.
- Sessions **+** and right-click **Add folder** / **Add device** stay inside the Late window.

## 2026-08-24

**Staging pane for CLI / Ansible / Netmiko / Salt / Chef drafts (operator Push only).**

- New Staging workspace pane (Tools → Staging). The helper can call `propose_staged_artifact` to write a draft on your computer. That tool does not log in, does not Push, and does not run playbooks.
- Daemon `stage.render|save|get|list|plan|push` stores files under Late data `staging/` (mode 0600). Drafts with password-like fields are rejected. Generic inventory does not assume Cisco IOS.
- **Push** is operator-only from Staging (click to confirm; Enter does not confirm). CLI types the draft into an SSH/serial session you already opened. Ansible / Netmiko / Salt / Chef run on your computer via PATH tools Late does not bundle (`ansible-playbook`, `python3`+netmiko, `salt-call`/`salt-ssh`, `chef-apply`). Generated inventory never contains passwords or `ansible_ssh_pass`. Key/agent stay in inventory extra args. Password-only auth from the daemon vault is injected at run time (`sshpass -e` + `SSHPASS` on the late-daemon child). Missing binaries toast a clear install error. The helper still cannot Push.
- PATH Push (Ansible / Netmiko / Salt / Chef) uses the **SSH inventory device** you pick (hostname/IP on your computer). Auth is that device’s profile, or the login an open SSH session to the same host already used. An open serial session in **Push session** is ignored — that dropdown is only for Push CLI into an open terminal.
- Same inventory and auth profiles as SSH. Generated drafts list platform/user intent only — no passwords in the files.

## 2026-08-20 15:52

**Keep SSH passwords working after device banners, fix Approve, and tighten the control plane.**

SSH

- Password SSH to hosts that print a pre-auth banner (Aruba “register your products”, and similar) no longer fails with `ssh_askpass: No such file or directory`. Late was deleting the ASKPASS helper as soon as `ssh` spawned; OpenSSH needs that file later, after the banner. The helper still cats the password, truncates it, and deletes itself when invoked. Leftovers are reaped after two minutes and swept on the next connect. The password is still not put in process environment (`SSHPASS` unused); files stay `0600` / `0700`.

Agent and shell

- Approve actually stays on screen until you Approve or Deny. Cursor is given MCP tools so `propose_command` runs; proposal ids are real UUIDs; the modal is not cleared in `finally`.
- Host-key trust is click-only (Enter does not confirm). Stop clears a pending proposal. Always-allow reuse requires an explicit always-ok.
- Sessions **+** menu is `position: fixed` so it is not clipped off a narrow sidebar.
- Panes can be resized and moved. Agent dock has **Wide** / **Top**. Models list is resizable/collapsible.
- Edit menu copy/paste is restored (clipboard stays in the main process).
- Settings **Cloud AI** switch matches `cloud_chat_enabled` (still off by default).

Control plane (ISMS aim, not a certificate)

- Sidecar `/health` is `{"ok":true}` only. Chat no longer puts the sidecar token in the query string. Cursor `send()` prefixes operator text as untrusted.
- Identity files stay under home but cannot be Late config/data or names like `sidecar.token` / `secrets.json`. Policy overlay cannot always-allow a builtin deny (including after Cisco `do`). `log_dir` is confined like `pcap_dir`. Inventory and auth profiles write `0600`.
- Electron denies webview permissions and fails closed on framed navigation. Release workflow is `contents: read` except the tagged publish job.
- Docs: dual-gate is sidecar-only; no BAA; Unix `0600` vs NTFS; `/health` honesty. Do not treat this as SOC 2 or ISO 27001 certification.

## 2026-08-20 12:31

**Finish installer packaging for Linux, macOS, and Windows.**

- `.deb` needs homepage, author email, and maintainer. macOS needs a 512×512 icon. Windows cannot compile Unix `chmod` on the sidecar token file.

## 2026-08-20 12:23

**Pin Electron so GitHub can build installers.**

- electron-builder requires an exact Electron version. Packaging now uses `--publish never` so a git tag does not trigger a second publisher.

## 2026-08-20 12:13

**Fix installer CI (`npm ci` lockfile) and make from-source the supported install.**

- Regenerated `package-lock.json` so GitHub can install dependencies on Linux, macOS, and Windows. Prebuilt apps still come from those runners — this Linux machine cannot emit a Mac `.dmg` or Windows `.exe`.

## 2026-08-20 12:08

**Publish installers on the GitHub Releases page.**

- Tagging `v*` now attaches Linux / macOS / Windows packages to a Release (the old workflow only stored Actions artifacts, which is why `/releases` was empty). First tagged build is `v0.1.0`. Installers are still unsigned.

## 2026-08-20 11:45

**SOC 2 / ISO 27001 ISMS (aim, not a certificate) and local-only chat by default.**

- Documented ISMS in [docs/isms/](docs/isms/README.md) (scope, policy, risk, SoA mapping Annex A to SOC 2 TSC, assets, suppliers, access, change, incident). Report issues via [SECURITY.md](SECURITY.md). Do not treat this as SOC 2 or ISO 27001 certification.
- `cloud_chat_enabled` defaults to false. Settings checkbox enables Cursor and non-loopback OpenAI-compatible chat; the toggle is audited. Hugging Face GGUF download and Ollama Pull stay available.
- `/chat` is audited (backend + loopback vs cloud, no bodies). `pcap_dir` cannot expand the path jail. Downloaded GGUF files must start with the `GGUF` magic. Google Fonts dropped from the UI CSP.

## 2026-08-20 11:13

**Recommend more SFW Hub families (Gemma, Llama, Mistral, Phi, Granite) alongside Qwen.**

- Agent pane catalogs for local vLLM, llama.cpp GGUF, and Ollama Pull include Google Gemma, Meta Llama, Mistral, Microsoft Phi, and IBM Granite instruct ids, plus Qwen. Uncensored/NSFW fine-tunes stay out of the list. Any other Hub id can still be typed. Gemma and Llama are gated — set `HF_TOKEN` after accepting the license.

## 2026-08-20 10:47

**Hugging Face download for llama.cpp, and Ollama Pull (including Hub ids).**

- llama.cpp Agent pane can Download GGUF from Hugging Face into `~/.local/share/late/models/gguf/` (no Intel Docker). Optional Start/Stop if `llama-server` is on PATH.
- Ollama Agent pane can Pull library names or Hugging Face ids (`hf.co/…`) through the local Ollama API on loopback. Late still does not install Ollama or llama.cpp.

## 2026-08-20 09:10

**Vendor-agnostic local inference, llama.cpp, and hide GPU controls when unused.**

- Docs no longer headline a specific Intel Arc card. NVIDIA, AMD, and Intel GPUs all work if the server you run uses them. `docker/compose.yml` is an optional Intel XPU vLLM example only.
- Agent pane and Settings show vLLM / GPU start-stop / Hugging Face download only when **local vLLM** is selected. Ollama, llama.cpp, and Cursor hide those personal-runtime controls. *(Superseded for llama.cpp Download and Ollama Pull by 10:47.)*
- Start / Download refuse NVIDIA and AMD (they would have pulled `intel/llm-scaler-vllm`). Use Ollama, llama.cpp, or your own CUDA/ROCm server. `LATE_VLLM_FORCE=1` overrides.
- First-class **llama.cpp** backend (`llama-server` at `http://127.0.0.1:8080/v1`). Ollama, Cursor SDK, and local vLLM remain available. Late still does not bundle CUDA, ROCm, llama.cpp, Ollama, or model weights.

## 2026-08-20 08:59

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

## 2026-08-19 16:23

**Stop Cursor hangs after a command, and let Settings scroll.**

- Cursor idle timer no longer treats pre-tool “I’ll check…” text as the answer, and no longer closes the stream while Approve is pending.
- Stream iterator is no longer polled twice at once (that deadlock froze the agent).
- After a silent stall, one hop writes the answer from device output.
- Vendor permit check is fail-closed (`allowed === true` required).
- Settings (and other tall dialogs) scroll inside the viewport.

## 2026-08-19 14:57 — `3bfa1c4`

**Add nested inventory folders so devices can be grouped by site.**

- Inventory sidebar is a collapsible tree (SecureCRT-style). Paths like `NYC/Core` nest by `/`.
- Create, rename, and delete folders from an in-app form (Electron has no `window.prompt`).
- Drag devices onto folders or Ungrouped. New SSH/serial/API sessions inherit the selected folder.
- Empty folders stay in the tree for site planning. Rename refuses collisions and moving a folder into itself.
- Cursor agent stops looping after it has an answer instead of proposing extra `show` commands.
- Local PTY, Packet capture, and Edgeshark sit idle on the sidebar background; hover/focus gets the light grey highlight.

## 2026-08-19 12:10 — `f8e8aa6`

**Keep the agent going after an approved command and hide vLLM when using Cursor.**

- After you Approve a command, the agent reads device output and continues the answer (still dual-gated).
- When the chat backend is Cursor SDK, the local vLLM model list is hidden.

## 2026-08-19 11:34 — `aa132e2`

**Add optional Ollama backend without bundling GPU runtimes.**

- First-class Ollama option in the agent pane (`http://127.0.0.1:11434/v1`).
- Installers still do not ship vLLM, llama.cpp, CUDA, or Ollama itself.

## 2026-08-19 11:12 — `c505bef`

**Add cross-platform Late installers and GitHub Actions.**

- Electron packages the UI, `late-daemon`, and agent sidecar.
- CI builds Linux (AppImage/deb), macOS, and Windows artifacts on `v*` tags.
