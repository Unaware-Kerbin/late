#!/usr/bin/env bash
# Start Late daemon (if built), agent sidecar, and Vite UI.
set -euo pipefail
export PATH="${HOME}/.local/bin:${PATH}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

DAEMON_ONLY=0
UI_ONLY=0
for arg in "$@"; do
  case "$arg" in
    --daemon-only) DAEMON_ONLY=1 ;;
    --ui-only) UI_ONLY=1 ;;
  esac
done

find_daemon() {
  local p
  for p in \
    "$ROOT/target/debug/late-daemon" \
    "$ROOT/target/release/late-daemon" \
    "$ROOT/crates/late-daemon/target/debug/late-daemon" \
    "$ROOT/crates/late-daemon/target/release/late-daemon"
  do
    if [[ -x "$p" ]]; then
      echo "$p"
      return 0
    fi
  done
  return 1
}

pids=()
cleanup() {
  for pid in "${pids[@]+"${pids[@]}"}"; do
    kill "$pid" 2>/dev/null || true
  done
}
trap cleanup EXIT INT TERM

start_daemon() {
  if DAEMON_BIN="$(find_daemon)"; then
    echo "late: starting daemon $DAEMON_BIN"
    "$DAEMON_BIN" --bind 127.0.0.1:7420 &
    pids+=($!)
  else
    echo "late: daemon binary not found (build with: cargo build -p late-daemon)"
    echo "late: UI will show connection errors until the daemon is listening on 127.0.0.1:7420"
    if [[ "$DAEMON_ONLY" -eq 1 ]]; then
      exit 1
    fi
  fi
}

if [[ "$DAEMON_ONLY" -eq 1 ]]; then
  start_daemon
  wait
  exit 0
fi

if [[ "$UI_ONLY" -eq 0 ]]; then
  start_daemon
  echo "late: starting agent sidecar :7430"
  npm run dev -w late-agent-sidecar &
  pids+=($!)
fi

echo "late: starting Vite UI :5173"
npm run dev -w late-desktop &
pids+=($!)

wait
