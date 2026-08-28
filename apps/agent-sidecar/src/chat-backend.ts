/** Sidecar chat backends. Cloud AI never remaps MCP onto the Late Cursor SDK. */
export type SidecarChatBackend =
  | "cursor"
  | "ollama"
  | "llamacpp"
  | "anthropic"
  | "gemini"
  | "azure"
  | "mcp"
  | "local";

/**
 * Pick the sidecar loop from the GUI/backend field.
 * `cloudAiEnabled` is ignored: Agent=MCP always stays on the orchestrator, including Cursor cloud speakers.
 */
export function resolveSidecarChatBackend(
  raw?: string,
  _cloudAiEnabled?: boolean,
): SidecarChatBackend {
  void _cloudAiEnabled;
  if (raw === "cursor") return "cursor";
  if (raw === "ollama") return "ollama";
  if (raw === "llamacpp" || raw === "llama.cpp" || raw === "llama-cpp" || raw === "llama_cpp") {
    return "llamacpp";
  }
  if (raw === "anthropic") return "anthropic";
  if (raw === "gemini") return "gemini";
  if (raw === "azure") return "azure";
  if (raw === "mcp") return "mcp";
  return "local";
}
