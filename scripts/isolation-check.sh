#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SIDECAR="$ROOT/apps/agent-sidecar"
if [[ ! -d "$SIDECAR" ]]; then
  echo "isolation-check: apps/agent-sidecar not present; OK"
  exit 0
fi
if hits=$(grep -RInE 'keyring|russh|secret' "$SIDECAR" \
    --include='*.ts' --include='*.tsx' --include='*.js' --include='*.mjs' --include='*.cjs' --include='*.json' \
    --exclude-dir=node_modules --exclude-dir=dist --exclude-dir=.git 2>/dev/null); then
  echo "FAIL: agent-sidecar must not reference keyring/russh/secret"
  echo "$hits"
  exit 1
fi
echo "isolation-check: OK"
