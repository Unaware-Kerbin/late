# Late installer artifacts

Pack output is **not** stored in git (`apps/desktop/release/` is gitignored). GitHub Actions **Build installers** runs `scripts/pack.sh` on `main` and on `v*` tags, then uploads workflow artifacts. A `v*` tag also attaches the same files to the GitHub Release.

Future installers are **new tags** (`v0.1.12`, then `v0.1.13`, …). Do not rewrite old tags.

Do not commit the 100MB+ binaries.

## Names (`scripts/pack.sh` + electron-builder `artifactName`)

| File | What it is |
|---|---|
| `Late-<ver>-linux-amd64.deb` | Debian / Ubuntu / Mint. Docker is Suggests (`docker.io`), not Required. |
| `Late-<ver>-linux-x86_64.rpm` | Fedora / RHEL / Rocky / Alma / openSUSE. Docker is Suggests, not Requires. |
| `Late-<ver>-linux-x64.pacman` | Arch (optional Docker optdepend). AppImage still works if this is missing. |
| `Late-<ver>-linux-x86_64.AppImage` | Any Linux; make it executable. |
| `Late-<ver>-win-x64.exe` | Windows NSIS (unsigned). |
| `Late-<ver>-win-x64.zip` | Windows unpacked dir. |
| `Late-<ver>-mac-arm64.zip` | macOS Apple silicon: `Late.app` inside, **unsigned**. |
| `Late-<ver>-mac-arm64.dmg` | Same, when macos-latest can build a dmg (unsigned, no notarization). |

Exact `os`/`arch` tokens follow electron-builder (`linux`/`win`/`mac`, `amd64`/`x86_64`/`x64`/`arm64`).

## Honest gaps

- Mac zip/dmg are unsigned (no Apple notarization). First open: right-click Late → Open.
- Windows is unsigned (no Authenticode). SmartScreen: More info → Run anyway.
- `late-daemon` is built on that OS runner. A Mac zip packed on Linux has no Mach-O daemon until **macos-latest** runs.
- Ollama + `llama-server` (Vulkan / Metal / mlx) are in the package. Model weights are not. vLLM Start still needs Docker on your computer.

## How CI publishes

1. Matrix: Ubuntu `--linux`, macOS `--mac`, Windows `--win`.
2. `actions/upload-artifact` keeps the files on the workflow run.
3. On a `v*` tag, the `publish` job attaches `dist/*` to the GitHub Release.
