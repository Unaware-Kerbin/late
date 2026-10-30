#!/usr/bin/env bash
# Unpack the Electron binary into node_modules (42+ no longer does this in postinstall).
# Do not execute that binary. `electron --version` still starts the Linux zygote,
# and a checkout's chrome-sandbox is not root:4755, so the probe aborts with
# SIGTRAP when user namespaces are unavailable.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

name=electron
case "$(uname -s)" in
  Darwin*) name="Electron.app/Contents/MacOS/Electron" ;;
  MINGW*|MSYS*|CYGWIN*) name=electron.exe ;;
esac
if [[ "${OS:-}" == Windows_NT && "$name" == electron ]]; then
  name=electron.exe
fi

pkg_dirs=(
  "$ROOT/node_modules/electron"
  "$ROOT/apps/desktop/node_modules/electron"
  "$ROOT/node_modules/late-desktop/node_modules/electron"
)

find_bin() {
  local pkg
  for pkg in "${pkg_dirs[@]}"; do
    if [[ -e "$pkg/dist/$name" ]]; then
      printf '%s\n' "$pkg/dist/$name"
      return 0
    fi
  done
  return 1
}

if find_bin; then
  exit 0
fi

echo "late: fetching Electron binary…" >&2
unpacked=0
for pkg in "${pkg_dirs[@]}"; do
  if [[ -f "$pkg/install.js" ]]; then
    node "$pkg/install.js"
    unpacked=1
    break
  fi
done
if [[ "$unpacked" -eq 0 ]]; then
  echo "late: electron package not installed (npm install in the repo)" >&2
  exit 1
fi

if find_bin; then
  exit 0
fi

echo "late: Electron binary missing after fetch (node_modules/electron/dist/$name)" >&2
exit 1
