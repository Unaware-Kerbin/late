#!/bin/bash
# Late .deb postinst. App still installs if Docker is declined.
set -e
echo "Late: Ollama and llama.cpp are included. Model weights are not — use Pull / Download on your computer."
if command -v docker >/dev/null 2>&1; then
  echo "Late: Docker is present. vLLM Start can pull an image on first use."
else
  echo "Late: Docker is optional for vLLM on your computer. Install with: apt install docker.io"
  echo "Late: then: usermod -aG docker \"${SUDO_USER:-$USER}\" and log out."
fi
# Fail-closed: no Docker as root unless debconf is present, loaded, and RET=true.
if [ -f /usr/share/debconf/confmodule ] && [ "${DEBIAN_FRONTEND:-}" != "noninteractive" ]; then
  if . /usr/share/debconf/confmodule; then
    db_set late/install-docker false || true
    db_input medium late/install-docker || true
    db_go || true
    RET=false
    db_get late/install-docker || true
    if [ "${RET:-false}" = "true" ]; then
      if command -v apt-get >/dev/null 2>&1; then
        apt-get install -y -qq docker.io || echo "Late: could not install docker.io. Install it yourself to use vLLM."
      fi
      if id "${SUDO_USER:-}" >/dev/null 2>&1; then
        usermod -aG docker "$SUDO_USER" || true
      fi
    fi
  fi
fi
