import { requestApproval } from "./approvals.js";
import { setGrantedMcpCwd } from "./mcp-chat-format.js";
import { mcpNeedsApprove } from "./mcp-format.js";

export const GRANT_DIR_HOST_NOTE =
  "Path must exist on the computer running Agent Orchestrator. A laptop-only folder is not copied.";
export const GRANT_DIR_MISSING_NOTE =
  "The folder must already exist on the MCP host. Late does not copy trees across machines.";

export type GrantDirBody = {
  ok?: boolean;
  cwd?: string;
  denied?: boolean;
  proposalId?: string;
  error?: string;
  output?: unknown;
  note?: string;
};

export type GrantDirResult = {
  status: number;
  body: GrantDirBody;
};

/** POST /mcp/grant-dir body. Bad JSON is handled by the HTTP layer as 400. */
export function parseGrantDirPath(body: unknown): { ok: true; path: string } | { ok: false; error: string } {
  if (body === null || typeof body !== "object" || Array.isArray(body)) {
    return { ok: false, error: "JSON object with path string required" };
  }
  const path = (body as { path?: unknown }).path;
  if (typeof path !== "string") {
    return { ok: false, error: "path string required" };
  }
  return { ok: true, path };
}

/**
 * Queue add_allowed_dir behind Late Approve, then call MCP. Never auto-calls the tool.
 * Path must already exist on the MCP host — Late does not SCP or copy trees.
 * Never throws — missing host path is JSON 400 so the sidecar process stays up.
 */
export async function grantMcpAllowedDir(
  rawPath: string,
  callTool: (name: string, args: Record<string, unknown>) => Promise<unknown>,
  onQueued?: (proposalId: string, path: string) => void,
): Promise<GrantDirResult> {
  const dirPath = rawPath.trim();
  if (!dirPath) {
    return { status: 400, body: { error: "path string required" } };
  }
  if (!mcpNeedsApprove("add_allowed_dir")) {
    return { status: 500, body: { error: "add_allowed_dir must stay Approve-gated" } };
  }
  const { proposal, promise } = requestApproval({
    kind: "command",
    title: "MCP: add_allowed_dir",
    detail: { command: "mcp add_allowed_dir", mcp: "add_allowed_dir", args: { path: dirPath } },
    allowAlwaysAllow: false,
  });
  onQueued?.(proposal.proposalId, dirPath);
  let decision: { allow: boolean };
  try {
    decision = await promise;
  } catch (err) {
    return {
      status: 200,
      body: {
        ok: false,
        denied: true,
        error: err instanceof Error ? err.message : String(err),
      },
    };
  }
  if (!decision.allow) {
    return { status: 200, body: { ok: false, denied: true, proposalId: proposal.proposalId } };
  }
  try {
    const output = await callTool("add_allowed_dir", { path: dirPath });
    setGrantedMcpCwd(dirPath);
    return {
      status: 200,
      body: {
        ok: true,
        cwd: dirPath,
        output,
        note: GRANT_DIR_HOST_NOTE,
      },
    };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    return {
      status: 400,
      body: {
        ok: false,
        error: message,
        note: GRANT_DIR_MISSING_NOTE,
      },
    };
  }
}
