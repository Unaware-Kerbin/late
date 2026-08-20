# ISMS scope

**Organization in scope:** maintainers of the Late project (GitHub `Unaware-Kerbin/late`), covering development, release, and the local runtime they operate while developing.

**Product in scope:**

- Late desktop UI (Electron; optional Tauri)
- `late-daemon` (JSON-RPC on loopback)
- Agent sidecar
- Vendor permit lists in `policies/`
- GitHub Actions for CI and unsigned installer builds

**Information in scope:** inventory, auth profiles, sidecar token, provider API keys, session logs/captures, packet captures, GGUF weights stored under Late data dirs, and GitHub repository contents.

**Out of scope:**

- Customer or operator workstations, MDM, disk encryption, and screen lock (organizational; required for a real SOC 2 / ISO engagement)
- Networks and devices Late SSHs into
- Cloud model providers’ own ISMS (Cursor, OpenAI, Anthropic, Groq, OpenRouter)
- Hugging Face Hub as a service (weight download only)
- Ollama, llama.cpp, and vLLM when installed and run by the operator
- Payment card data (Late does not process CHD)
- HIPAA / ePHI program (not this ISMS)

**Locations:** developer workstations and GitHub-hosted CI runners. Production is the operator’s machine after they install Late.

**Statement:** Late aims to implement technical controls that map to ISO 27001 Annex A and SOC 2 TSC. Maintainers do not claim certification.
