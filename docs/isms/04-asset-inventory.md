# Asset inventory

Config and data directories are the platform dirs named `late` (`dirs::config_dir` / `dirs::data_dir`). Typical locations:

| OS | Config | Data |
|---|---|---|
| Linux | `~/.config/late/` | `~/.local/share/late/` |
| macOS | `~/Library/Application Support/late/` | `~/Library/Application Support/late/` |
| Windows | `%APPDATA%\late` | `%APPDATA%\late` |

On Unix, secrets and tokens are mode 0600 and config/data dirs are 0700. Windows uses default NTFS ACLs on the config/data dirs.

| Asset | Type | Location | Owner | Classification |
|---|---|---|---|---|
| late-daemon | software | repo `crates/late-daemon`, runtime on operator host | maintainers | confidential (holds secrets) |
| agent sidecar | software | `apps/agent-sidecar` | maintainers | confidential (token, provider keys in memory) |
| Electron UI | software | `apps/desktop` | maintainers | internal |
| Electron IPC | control | `late:token`, `late:clipboard-read`, `late:clipboard-write`; Vite `/__late_token` in dev | maintainers | secret (token) / confidential (clipboard). Origin-checked in main; not audited |
| sidecar.token | secret | config dir `sidecar.token` | operator | secret |
| secrets.json | secret | config dir `secrets.json` | operator | secret |
| provider-keys.json | secret | config dir `provider-keys.json` | operator | secret |
| inventory / auth TOML | config | config dir | operator | confidential |
| audit.jsonl | log | data dir `audit.jsonl` | operator | confidential |
| session logs / captures / pcap | data | data dir | operator | confidential |
| GGUF weights | data | data dir `models/gguf/` | operator | internal (license-bound) |
| GitHub repo | source | `Unaware-Kerbin/late` | maintainers | public source; no production secrets |
| GitHub Actions | CI | `.github/workflows/` | maintainers | internal |
| policies/*.yaml | control | repo + config dir `policies/` | maintainers | internal |

Personal data: only if the operator pastes it into a session or chat. Late does not require an account.
