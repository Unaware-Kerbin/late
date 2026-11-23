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
# Linux extracts chrome-sandbox without the setuid bit, and a bare
# `electron --version` aborts before it prints a version. macOS and
# Windows keep the sandbox for this probe.
case "$(uname -s)" in
  Linux*)
    export ELECTRON_DISABLE_SANDBOX=1
    npm exec -w late-desktop -- electron --no-sandbox --version >/dev/null || true
    ;;
  *)
    npm exec -w late-desktop -- electron --version >/dev/null || true
    ;;
esac

for dir in "$ROOT/node_modules/electron/dist" "$ROOT/apps/desktop/node_modules/electron/dist"; do
  if [[ -e "$dir/$name" ]]; then
    printf '%s\n' "$dir/$name"
    exit 0
  fi
done

echo "late: Electron binary missing after fetch (node_modules/electron/dist/$name)" >&2
exit 1
