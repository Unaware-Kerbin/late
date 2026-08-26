#!/usr/bin/env bash
# Print the Electron dist binary path. Electron 42+ npm postinstall does not unzip it.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

find_electron() {
  local p
  for p in \
    "$ROOT/node_modules/electron/dist/electron" \
    "$ROOT/apps/desktop/node_modules/electron/dist/electron" \
    "$ROOT/node_modules/electron/dist/electron.exe" \
    "$ROOT/apps/desktop/node_modules/electron/dist/electron.exe"
  do
    if [[ -f "$p" ]]; then
      echo "$p"
      return 0
    fi
  done
  return 1
}

if found="$(find_electron)"; then
  echo "$found"
  exit 0
fi

echo "late: fetching Electron binary (npm postinstall no longer unzips dist)…" >&2
npm exec -w late-desktop -- electron --version >&2

if found="$(find_electron)"; then
  echo "$found"
  exit 0
fi

echo "late: Electron binary missing after fetch (looked in node_modules/electron/dist)" >&2
exit 1
