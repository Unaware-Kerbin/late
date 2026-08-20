# Asset inventory

| Asset | Type | Location | Owner | Classification |
|---|---|---|---|---|
| late-daemon | software | repo `crates/late-daemon`, runtime on operator host | maintainers | confidential (holds secrets) |
| agent sidecar | software | `apps/agent-sidecar` | maintainers | confidential (token, provider keys in memory) |
| Electron UI | software | `apps/desktop` | maintainers | internal |
| sidecar.token | secret | `~/.config/late/sidecar.token` | operator | secret |
| secrets.json | secret | `~/.config/late/secrets.json` | operator | secret |
| provider-keys.json | secret | `~/.config/late/provider-keys.json` | operator | secret |
| inventory / auth TOML | config | `~/.config/late/` | operator | confidential |
| audit.jsonl | log | `~/.local/share/late/audit.jsonl` | operator | confidential |
| session logs / captures / pcap | data | `~/.local/share/late/` | operator | confidential |
| GGUF weights | data | `~/.local/share/late/models/gguf/` | operator | internal (license-bound) |
| GitHub repo | source | `Unaware-Kerbin/late` | maintainers | public source; no production secrets |
| GitHub Actions | CI | `.github/workflows/` | maintainers | internal |
| policies/*.yaml | control | repo + `~/.config/late/policies/` | maintainers | internal |

Personal data: only if the operator pastes it into a session or chat. Late does not require an account.
