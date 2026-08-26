#!/usr/bin/env bash
# Unpack the Electron binary into node_modules (42+ no longer does this in postinstall).
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

for dir in "$ROOT/node_modules/electron/dist" "$ROOT/apps/desktop/node_modules/electron/dist"; do
  if [[ -e "$dir/$name" ]]; then
    printf '%s\n' "$dir/$name"
    exit 0
  fi
done

echo "late: fetching Electron binary…" >&2
npm exec -w late-desktop -- electron --version >/dev/null

for dir in "$ROOT/node_modules/electron/dist" "$ROOT/apps/desktop/node_modules/electron/dist"; do
  if [[ -e "$dir/$name" ]]; then
    printf '%s\n' "$dir/$name"
    exit 0
  fi
done

echo "late: Electron binary missing after fetch (node_modules/electron/dist/$name)" >&2
exit 1
