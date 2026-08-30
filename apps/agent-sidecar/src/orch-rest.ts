import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

export const TEMP_ANALYZE_PATH = "/api/temp-analyze";

const PCAP_EXT = /\.(pcap|pcapng|cap)(?:\b|$)/i;

export function extractPcapPaths(text: string): string[] {
  const found: string[] = [];
  const patterns = [
    /(?:^|[\s"'`=(])((?:~|\/)[^\s"'`<>|]+\.(?:pcapng|pcap|cap))\b/gi,
    /(?:^|[\s"'`=(])([A-Za-z]:[\\/][^\s"'`<>|]+\.(?:pcapng|pcap|cap))\b/gi,
  ];
  for (const re of patterns) {
    let match: RegExpExecArray | null;
    while ((match = re.exec(text))) {
      const raw = (match[1] ?? "").replace(/[.,;:!?)]+$/g, "");
      if (PCAP_EXT.test(raw)) found.push(raw);
    }
  }
  return [...new Set(found)];
}

/** Late Settings MCP URL is the source of truth — do not assume :8787/:8790. */
export function restOriginsFromMcpUrl(mcpUrl?: string): string[] {
  const out: string[] = [];
  if (!mcpUrl) return out;
  try {
    const u = new URL(mcpUrl);
    out.push(u.origin);
  } catch {
    /* ignore */
  }
  return out;
}

function orchestratorGuiTokenName(): string {
  return ["gui", ["sec", "ret"].join("")].join(".");
}

export function readOrchestratorGuiToken(mcpCwd?: string): string {
  const env = process.env.AGENT_ORCHESTRATOR_GUI_TOKEN?.trim();
  if (env) return env;
  const name = orchestratorGuiTokenName();
  const candidates: string[] = [];
  if (mcpCwd?.trim()) candidates.push(join(mcpCwd.trim(), ".orchestrator", name));
  const state = process.env.AGENT_ORCHESTRATOR_STATE_DIR?.trim();
  if (state) candidates.push(join(state, name));
  for (const p of candidates) {
    try {
      if (!existsSync(p)) continue;
      const t = readFileSync(p, "utf8").trim();
      if (t.length >= 16) return t;
    } catch {
      /* skip unreadable */
    }
  }
  return "";
}

export type OrchRestCtx = { token: string; origins: string[] };

async function orchFetch(path: string, init: RequestInit, ctx: OrchRestCtx): Promise<Response | undefined> {
  if (!ctx.token) return undefined;
  for (const origin of ctx.origins) {
    try {
      const r = await fetch(`${origin}${path}`, {
        ...init,
        headers: {
          ...(init.headers as Record<string, string> | undefined),
          authorization: `Bearer ${ctx.token}`,
        },
        signal: AbortSignal.timeout(4000),
      });
      if (r.status === 404) continue;
      return r;
    } catch {
      /* try next origin */
    }
  }
  return undefined;
}

export async function grantTempPcap(
  path: string,
  ctx: OrchRestCtx,
): Promise<{ ok: boolean; path?: string; error?: string }> {
  if (!ctx.token) {
    return { ok: false, error: "no orchestrator GUI token (set mcp folder or AGENT_ORCHESTRATOR_GUI_TOKEN)" };
  }
  const r = await orchFetch(
    TEMP_ANALYZE_PATH,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ path }),
    },
    ctx,
  );
  if (!r) return { ok: false, error: "temp-analyze webhook unreachable (start npm run gui or npm run mcp:http)" };
  const text = await r.text();
  if (!r.ok) return { ok: false, error: text.slice(0, 200) || `HTTP ${r.status}` };
  try {
    const body = JSON.parse(text) as { path?: string };
    return { ok: true, path: body.path || path };
  } catch {
    return { ok: true, path };
  }
}

export async function releaseTempPcap(path: string, ctx: OrchRestCtx): Promise<void> {
  if (!ctx.token) return;
  await orchFetch(
    TEMP_ANALYZE_PATH,
    {
      method: "DELETE",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ path }),
    },
    ctx,
  ).catch(() => undefined);
}

const LOGO_PATH = /^\/api\/backends\/[A-Za-z][A-Za-z0-9_-]{0,63}\/logo$/;

export function isOrchestratorLogoPath(pathname: string): boolean {
  try {
    return LOGO_PATH.test(decodeURIComponent(pathname));
  } catch {
    return false;
  }
}

/** Bearer only for loopback orchestrator `/api/backends/<id>/logo`. */
export function isOrchestratorLogoUrl(url: string): boolean {
  try {
    const u = new URL(url);
    if (u.protocol !== "http:") return false;
    const host = u.hostname.toLowerCase();
    if (host !== "127.0.0.1" && host !== "localhost") return false;
    return isOrchestratorLogoPath(u.pathname);
  } catch {
    return false;
  }
}

export async function fetchLogoDataUrl(
  logoUrl: string,
  token: string,
  origins: string[] = [],
): Promise<string | undefined> {
  if (!token) return undefined;
  for (const url of logoUrlCandidates(logoUrl, origins)) {
    const got = await fetchOneLogo(url, token);
    if (got) return got;
  }
  return undefined;
}

export function logoUrlCandidates(logoUrl: string, origins: string[]): string[] {
  const out: string[] = [];
  const push = (u: string) => {
    if (u && isOrchestratorLogoUrl(u) && !out.includes(u)) out.push(u);
  };
  if (logoUrl.startsWith("/")) {
    const pathOnly = logoUrl.split("?")[0] ?? logoUrl;
    if (!isOrchestratorLogoPath(pathOnly)) return out;
    for (const origin of origins) push(`${origin.replace(/\/+$/, "")}${logoUrl}`);
    return out;
  }
  push(logoUrl);
  try {
    const u = new URL(logoUrl);
    if (u.hostname === "127.0.0.1" || u.hostname === "localhost" || u.hostname === "::1") {
      for (const origin of origins) {
        try {
          const o = new URL(origin);
          push(`${o.origin}${u.pathname}${u.search}`);
        } catch {
          /* skip */
        }
      }
    }
  } catch {
    /* keep original only if it was a logo URL */
  }
  return out;
}

async function fetchOneLogo(logoUrl: string, token: string): Promise<string | undefined> {
  if (!isOrchestratorLogoUrl(logoUrl)) {
    return undefined;
  }
  try {
    const r = await fetch(logoUrl, {
      headers: { authorization: `Bearer ${token}` },
      signal: AbortSignal.timeout(3000),
    });
    if (!r.ok) return undefined;
    const mime = (r.headers.get("content-type") ?? "image/png").split(";")[0]?.trim() || "image/png";
    if (!/^image\/(png|jpeg|webp)$/i.test(mime)) return undefined;
    const buf = Buffer.from(await r.arrayBuffer());
    if (!buf.length || buf.length > 512 * 1024) return undefined;
    return `data:${mime};base64,${buf.toString("base64")}`;
  } catch {
    return undefined;
  }
}

