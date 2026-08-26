# Supplier register

| Supplier | Purpose | Data | Cloud AI? | Residual |
|---|---|---|---|---|
| Cursor / Anysphere (`@cursor/sdk`) | Optional agent backend | Prompts, scrollback, tool results when enabled | Yes, after Settings opt-in | Needs DPA if used with regulated data (HIPAA is out of scope) |
| OpenAI, Anthropic, Groq, OpenRouter, Gemini, Azure, custom OpenAI-compatible | Optional Bearer / vendor key to a public or private chat URL | Prompts and scrollback when that URL is used | Public hosts after Cloud AI opt-in; private RFC1918 / `.internal` without that toggle | Same |
| Hugging Face | GGUF list/download; gated `HF_TOKEN` | Model bytes, token; not session text | No | Supply chain; GGUF magic check |
| Ollama, Inc. | Optional local or operator-run server; Pull hits registry | Models; chat stays loopback or private-network unless URL is public | Only if operator points at a public host **and** opt-in | Operator-run |
| GitHub | Source, CI, release artifacts | Public repo, Actions logs | No | Unsigned artifacts this pass |
| npm / crates.io | Dependencies | Build-time | No | Advisory scans in CI |
| Intel (`llm-scaler-vllm` image) | Optional compose example | Weights via HF_TOKEN in container | No | Privileged Docker; NVIDIA/AMD Start refused |
| Electron / Tauri | Desktop shell | Local | No | `--no-sandbox` on source launchers (`./late`, `npm run native`). Packaged Electron-builder binaries do not set the flag |

Google Fonts: **removed** from the UI CSP (no longer a runtime supplier).

Review: 2026-08-20.
