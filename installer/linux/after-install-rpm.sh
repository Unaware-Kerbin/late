#!/bin/bash
# Late rpm/pacman postinst. Does not install Docker (Suggests/optdepends only).
set -e
echo "Late: Ollama and llama.cpp are included. Model weights are not — use Pull / Download on your computer."
if command -v docker >/dev/null 2>&1; then
  echo "Late: Docker is present. vLLM Start can pull an image on first use."
else
  echo "Late: Docker is optional for vLLM on your computer. Install it yourself if you want vLLM Start."
fi
