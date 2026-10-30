#!/usr/bin/env bash
# Install clab-ssh next to Late. Does not rebuild Late's packages and does not
# change how the packaged app starts Electron.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="${1:-${CLAB_SSH_SRC:-}}"

if [[ -z "$SRC" ]]; then
  for candidate in \
    "$ROOT/../clab-terminal-launcher" \
    "$HOME/Documents/clab-terminal-launcher" \
    "$HOME/src/clab-terminal-launcher"
  do
    if [[ -f "$candidate/install.sh" && -f "$candidate/clab-ssh" ]]; then
      SRC="$candidate"
      break
    fi
  done
fi

if [[ -z "$SRC" || ! -f "$SRC/install.sh" || ! -f "$SRC/clab-ssh" ]]; then
  echo "late: clab-ssh sources not found." >&2
  echo "late: clone https://github.com/Unaware-Kerbin/clab-terminal-launcher" >&2
  echo "late: then run ./install.sh in that checkout (Windows: .\\install.ps1)" >&2
  echo "late: or: $ROOT/installer/install-clab-ssh.sh /path/to/clab-terminal-launcher" >&2
  exit 1
fi

SRC="$(cd "$SRC" && pwd)"
echo "late: installing clab-ssh from $SRC"
exec bash "$SRC/install.sh"
