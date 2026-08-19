# Changelog

Dates and times are from git commits (America/New_York). Newest first.

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
