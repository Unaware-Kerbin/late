/** When vLLM Start / Download show on this computer (loopback + Intel compose + Docker). */
export function vllmStartVisible(opts: {
  allowIntelCompose: boolean;
  dockerAvailable: boolean;
  networkServer: boolean;
}): boolean {
  return !opts.networkServer && opts.allowIntelCompose && opts.dockerAvailable;
}

export function vllmDockerHint(opts: {
  dockerAvailable: boolean;
  allowIntelCompose: boolean;
  networkServer: boolean;
}): string | null {
  if (opts.networkServer) return null;
  if (!opts.dockerAvailable) {
    return "Install Docker to use vLLM on this computer. Ollama and llama.cpp are included and do not need Docker.";
  }
  return null;
}
