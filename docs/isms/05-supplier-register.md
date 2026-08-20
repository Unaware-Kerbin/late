# Supplier register

| Supplier | Purpose | Data | Cloud chat? | Residual |
|---|---|---|---|---|
| Cursor / Anysphere (`@cursor/sdk`) | Optional agent backend | Prompts, scrollback, tool results when enabled | Yes, after Settings opt-in | Needs DPA/BAA if used with regulated data |
| OpenAI, Anthropic, Groq, OpenRouter, custom OpenAI-compatible | Optional Bearer to non-loopback vLLM URL | Same as above when URL is not loopback | Yes, after opt-in | Same |
| Hugging Face | GGUF list/download; gated `HF_TOKEN` | Model bytes, token; not session text | No | Supply chain; GGUF magic check |
| Ollama, Inc. | Optional local server; Pull hits registry | Models; chat stays loopback unless URL changed | Only if operator points off-loopback **and** opt-in | Operator-run |
| GitHub | Source, CI, release artifacts | Public repo, Actions logs | No | Unsigned artifacts this pass |
| npm / crates.io | Dependencies | Build-time | No | Advisory scans in CI |
| Intel (`llm-scaler-vllm` image) | Optional compose example | Weights via HF_TOKEN in container | No | Privileged Docker; NVIDIA/AMD Start refused |
| Electron / Tauri | Desktop shell | Local | No | `--no-sandbox` residual on Linux launchers |

Google Fonts: **removed** from the UI CSP (no longer a runtime supplier).

Review: 2026-08-20.
