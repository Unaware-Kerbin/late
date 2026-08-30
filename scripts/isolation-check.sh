#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SIDECAR="$ROOT/apps/agent-sidecar"
if [[ ! -d "$SIDECAR" ]]; then
  echo "isolation-check: apps/agent-sidecar not present; OK"
  exit 0
fi
# Tests may name get_secrets / /secret so they can prove those stay gated.
EXCLUDE_TESTS=(--exclude='*.test.ts' --exclude='*.test.tsx' --exclude='*.test.js' --exclude='*.test.mjs' --exclude='*.test.cjs')
if hits=$(grep -RInE 'keyring|russh|secret' "$SIDECAR" \
    --include='*.ts' --include='*.tsx' --include='*.js' --include='*.mjs' --include='*.cjs' --include='*.json' \
    "${EXCLUDE_TESTS[@]}" \
    --exclude-dir=node_modules --exclude-dir=dist --exclude-dir=.git 2>/dev/null); then
  echo "FAIL: agent-sidecar must not reference keyring/russh/secret"
  echo "$hits"
  exit 1
fi
if hits=$(grep -RInE 'stage\.push|stage\.plan' "$SIDECAR" \
    --include='*.ts' --include='*.tsx' --include='*.js' --include='*.mjs' --include='*.cjs' \
    "${EXCLUDE_TESTS[@]}" \
    --exclude-dir=node_modules --exclude-dir=dist --exclude-dir=.git 2>/dev/null); then
  echo "FAIL: agent-sidecar must not call stage.push or stage.plan"
  echo "$hits"
  exit 1
fi
if hits=$(grep -RInE 'sftp\.(get|put|upload|download)|scp\.(get|put|upload|download)' "$SIDECAR" \
    --include='*.ts' --include='*.tsx' --include='*.js' --include='*.mjs' --include='*.cjs' \
    "${EXCLUDE_TESTS[@]}" \
    --exclude-dir=node_modules --exclude-dir=dist --exclude-dir=.git 2>/dev/null); then
  echo "FAIL: agent-sidecar must not call file-transfer RPCs"
  echo "$hits"
  exit 1
fi
echo "isolation-check: OK"
