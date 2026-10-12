import type { ChatBackend } from "../types";
import type { ChatEgress, ChatTargetMeta } from "./sidecar";

/** Sync egress for UI gating (no DNS). Same private suffixes / ranges as the sidecar. */
const PRIVATE_SUFFIXES = [".local", ".internal", ".lan", ".home.arpa"] as const;

function normalizeHost(host: string): string {
  return host.trim().toLowerCase().replace(/^\[|\]$/g, "").replace(/\.$/, "");
}

function parseExtraHosts(raw: string): string[] {
  if (!raw.trim()) return [];
  const out: string[] = [];
  const seen = new Set<string>();
  for (const part of raw.split(/[\s,]+/)) {
    const host = normalizeHost(part);
    if (!host || host === "*" || host.includes("/") || host.includes("..")) continue;
    if (seen.has(host)) continue;
    seen.add(host);
    out.push(host);
  }
  return out;
}

function isIpv4PrivateOrLoopback(ip: string): ChatEgress | null {
  const m = ip.match(/^(\d+)\.(\d+)\.(\d+)\.(\d+)$/);
  if (!m) return null;
  const a = Number(m[1]);
  const b = Number(m[2]);
  if (a === 127) return "loopback";
  if (a === 10) return "private";
  if (a === 192 && b === 168) return "private";
  if (a === 172 && b >= 16 && b <= 31) return "private";
  return "cloud";
}

function classifyHostname(host: string, extraHosts: readonly string[]): ChatEgress {
  const h = normalizeHost(host);
  if (!h) return "cloud";
  if (extraHosts.some((e) => normalizeHost(e) === h)) return "private";
  if (h === "localhost" || h === "localhost.localdomain") return "loopback";
  if (h === "::1") return "loopback";
  const v4 = isIpv4PrivateOrLoopback(h);
  if (v4) return v4;
  if (PRIVATE_SUFFIXES.some((s) => h.endsWith(s))) return "private";
  return "cloud";
}

/** Sync URL classify for Settings / Agent menus. Fail closed to cloud. */
export function classifyChatBaseSync(base: string, privateHosts = ""): ChatEgress {
  const trimmed = base.trim();
  if (!trimmed) return "cloud";
  try {
    const u = new URL(trimmed);
    if (u.protocol !== "http:" && u.protocol !== "https:") return "cloud";
    return classifyHostname(u.hostname, parseExtraHosts(privateHosts));
  } catch {
    return "cloud";
  }
}

export function chatEgressAllowed(egress: ChatEgress, cloudAiEnabled: boolean): boolean {
  return egress === "loopback" || egress === "private" || cloudAiEnabled;
}

/**
 * True when Cloud AI must be on to use this backend with the given target/URL.
 * Cursor is always cloud. Loopback/private URLs (incl. MCP folder) do not need Cloud AI.
 */
export function backendNeedsCloudAi(
  backend: ChatBackend,
  opts?: {
    egress?: ChatEgress;
    baseUrl?: string;
    privateHosts?: string;
    /** MCP folder/stdio with no HTTP URL stays local. */
    mcpStdio?: boolean;
  },
): boolean {
  if (backend === "cursor") return true;
  if (opts?.egress) return opts.egress === "cloud";
  if (backend === "mcp" && (opts?.mcpStdio || !opts?.baseUrl?.trim())) return false;
  if (opts?.baseUrl != null) {
    const egress = classifyChatBaseSync(opts.baseUrl, opts.privateHosts ?? "");
    return egress === "cloud";
  }
  // No target yet: SaaS defaults for native adapters; local engines stay available.
  return backend === "anthropic" || backend === "gemini" || backend === "azure";
}

export function cloudOffOptionSuffix(needsCloud: boolean, cloudAiEnabled: boolean): string {
  return needsCloud && !cloudAiEnabled ? " — Cloud AI is off" : "";
}

/** Prefer sidecar target egress; fall back to URL / defaults. */
export function backendBlockedWhenCloudOff(
  backend: ChatBackend,
  cloudAiEnabled: boolean,
  target?: ChatTargetMeta,
  fallback?: { baseUrl?: string; privateHosts?: string; mcpStdio?: boolean },
): boolean {
  if (cloudAiEnabled) return false;
  return backendNeedsCloudAi(backend, {
    egress: target?.egress,
    baseUrl: fallback?.baseUrl ?? target?.base,
    privateHosts: fallback?.privateHosts,
    mcpStdio: fallback?.mcpStdio,
  });
}
