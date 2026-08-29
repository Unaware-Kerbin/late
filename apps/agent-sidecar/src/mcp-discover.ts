import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { classifyChatBaseSync } from "./chat-target.js";
import { parseMcpHttpUrl } from "./mcp-format.js";

const DEFAULT_GUI_PORT = 8787;
const DEFAULT_MCP_PORT = 8790;
const MCP_URL_BASENAME = "mcp.url";
const MCP_GUI_URL_BASENAME = "mcp.gui.url";
const MCP_HTTP_URL_BASENAME = "mcp.http.url";

function loopbackMcp(port: number): string {
  return `http://127.0.0.1:${port}/mcp`;
}

function portFromEnv(raw: string | undefined, fallback: number): number {
  if (raw === undefined || raw.trim() === "") return fallback;
  const n = Number(raw);
  if (!Number.isInteger(n) || n < 1 || n > 65535) return fallback;
  return n;
}

function xdgOrchestratorDir(env: NodeJS.Dict<string>, home: string): string {
  const xdg = env.XDG_CONFIG_HOME?.trim();
  return xdg ? join(xdg, "agent-orchestrator") : join(home, ".config", "agent-orchestrator");
}

/** First line of an advertised mcp.url file, if it is loopback Streamable HTTP. */
export function parseAdvertisedMcpUrl(raw: string): string | undefined {
  const line = raw.trim().split(/\n/)[0]?.trim() ?? "";
  if (!line) return undefined;
  const parsed = parseMcpHttpUrl(line);
  if (typeof parsed !== "string") return undefined;
  if (classifyChatBaseSync(parsed) !== "loopback") return undefined;
  return parsed;
}

function readAdvertisedFile(
  path: string,
  readFile: (p: string) => string,
  exists: (p: string) => boolean,
): string | undefined {
  if (!exists(path)) return undefined;
  try {
    return parseAdvertisedMcpUrl(readFile(path));
  } catch {
    return undefined;
  }
}

function addAdvertisedDir(
  dir: string | undefined,
  add: (raw: string | undefined) => void,
  readFile: (path: string) => string,
  exists: (path: string) => boolean,
): void {
  if (!dir?.trim()) return;
  add(readAdvertisedFile(join(dir, MCP_GUI_URL_BASENAME), readFile, exists));
  add(readAdvertisedFile(join(dir, MCP_HTTP_URL_BASENAME), readFile, exists));
  add(readAdvertisedFile(join(dir, MCP_URL_BASENAME), readFile, exists));
}

/**
 * Loopback `/mcp` URLs to try when the Settings address is down.
 * Settings URL, then the URL the orchestrator wrote when it bound (not a hardcoded port),
 * then GUI port env / default, then dedicated mcp:http.
 */
export function mcpDiscoverCandidates(opts?: {
  settingsUrl?: string;
  mcpCwd?: string;
  env?: NodeJS.Dict<string>;
  homedir?: string;
  readFile?: (path: string) => string;
  exists?: (path: string) => boolean;
}): string[] {
  const env = opts?.env ?? process.env;
  const home = opts?.homedir ?? homedir();
  const readFile = opts?.readFile ?? ((p: string) => readFileSync(p, "utf8"));
  const exists = opts?.exists ?? existsSync;
  const out: string[] = [];
  const add = (raw: string | undefined) => {
    if (!raw?.trim()) return;
    const parsed = parseMcpHttpUrl(raw.trim());
    if (typeof parsed !== "string") return;
    if (classifyChatBaseSync(parsed) !== "loopback") return;
    if (!out.includes(parsed)) out.push(parsed);
  };
  add(opts?.settingsUrl);
  const cwd = opts?.mcpCwd?.trim();
  if (cwd) addAdvertisedDir(join(cwd, ".orchestrator"), add, readFile, exists);
  addAdvertisedDir(env.AGENT_ORCHESTRATOR_STATE_DIR?.trim(), add, readFile, exists);
  addAdvertisedDir(xdgOrchestratorDir(env, home), add, readFile, exists);
  add(loopbackMcp(portFromEnv(env.AGENT_ORCHESTRATOR_GUI_PORT, DEFAULT_GUI_PORT)));
  add(loopbackMcp(portFromEnv(env.AGENT_ORCHESTRATOR_MCP_PORT, DEFAULT_MCP_PORT)));
  return out;
}

export async function firstReachableMcpUrl(
  candidates: string[],
  probe: (url: string) => Promise<{ ok: boolean }>,
  skip: ReadonlySet<string> = new Set(),
): Promise<string | undefined> {
  for (const url of candidates) {
    if (skip.has(url)) continue;
    try {
      const probed = await probe(url);
      if (probed.ok) return url;
    } catch {
      /* try the next loopback /mcp */
    }
  }
  return undefined;
}
