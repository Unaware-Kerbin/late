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
if [[ -f "$ROOT/target/release/late-daemon.exe" ]]; then
  cp "$ROOT/target/release/late-daemon.exe" "$BIN_DIR/late-daemon.exe"
else
  cp "$ROOT/target/release/late-daemon" "$BIN_DIR/late-daemon"
  chmod +x "$BIN_DIR/late-daemon"
fi

echo "late: bundle sidecar"
npx --yes esbuild "$ROOT/apps/agent-sidecar/src/index.ts" \
  --bundle --platform=node --format=cjs \
  --external:@cursor/sdk \
  --outfile="$ROOT/apps/desktop/resources/sidecar.cjs"

echo "late: vite UI"
npm run build -w late-desktop

echo "late: electron-builder ${TARGET:-current OS}"
cd "$ROOT/apps/desktop"
npx --yes electron-builder ${TARGET:+$TARGET}
echo "late: artifacts in apps/desktop/release/"
