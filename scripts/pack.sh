#!/usr/bin/env bash
# Build Late installers for the current OS (or the electron-builder target passed in).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
TARGET="${1:-}"

echo "late: cargo release daemon"
cargo build -p late-daemon --release

BIN_DIR="$ROOT/apps/desktop/resources/bin"
mkdir -p "$BIN_DIR"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
if [[ -f "$TARGET_DIR/release/late-daemon.exe" ]]; then
  cp "$TARGET_DIR/release/late-daemon.exe" "$BIN_DIR/late-daemon.exe"
elif [[ -f "$TARGET_DIR/release/late-daemon" ]]; then
  cp "$TARGET_DIR/release/late-daemon" "$BIN_DIR/late-daemon"
  chmod +x "$BIN_DIR/late-daemon"
else
  echo "late: daemon binary missing under $TARGET_DIR/release" >&2
  exit 1
fi

echo "late: bundle sidecar"
npx --yes esbuild "$ROOT/apps/agent-sidecar/src/index.ts" \
  --bundle --platform=node --format=cjs \
  --external:@cursor/sdk \
  --outfile="$ROOT/apps/desktop/resources/sidecar.cjs"

echo "late: vite UI"
npm run build -w late-desktop

echo "late: ensure Electron binary"
"$ROOT/scripts/ensure-electron.sh"

echo "late: electron-builder ${TARGET:-current OS}"
if [[ -n "$TARGET" ]]; then
  npm exec -w late-desktop -- electron-builder --publish never "$TARGET"
else
  npm exec -w late-desktop -- electron-builder --publish never
fi
echo "late: artifacts in apps/desktop/release/"
