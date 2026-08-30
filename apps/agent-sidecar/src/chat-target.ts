import { promises as dns } from "node:dns";
import { BlockList, isIP } from "node:net";

/** Where chat tokens may go. `cloud` is public internet / unknown. */
export type ChatEgress = "loopback" | "private" | "cloud";

/** Loopback, RFC1918, ULA, and extra hosts do not need Cloud AI. Public, 169.254/16, and 0.0.0.0/8 do. */
export function chatEgressAllowed(egress: ChatEgress, cloud: boolean): boolean {
  return egress === "loopback" || egress === "private" || cloud;
}

const LOOPBACK = new BlockList();
LOOPBACK.addSubnet("127.0.0.0", 8, "ipv4");
LOOPBACK.addAddress("::1", "ipv6");
LOOPBACK.addSubnet("::ffff:127.0.0.0", 104, "ipv6");

const PRIVATE = new BlockList();
PRIVATE.addSubnet("10.0.0.0", 8, "ipv4");
PRIVATE.addSubnet("172.16.0.0", 12, "ipv4");
PRIVATE.addSubnet("192.168.0.0", 16, "ipv4");
PRIVATE.addSubnet("fc00::", 7, "ipv6");
PRIVATE.addSubnet("fe80::", 10, "ipv6");
PRIVATE.addSubnet("::ffff:10.0.0.0", 104, "ipv6");
PRIVATE.addSubnet("::ffff:172.16.0.0", 108, "ipv6");
PRIVATE.addSubnet("::ffff:192.168.0.0", 112, "ipv6");

const PRIVATE_SUFFIXES = [".local", ".internal", ".lan", ".home.arpa"] as const;

export function parseExtraHosts(raw: unknown): string[] {
  if (typeof raw !== "string" || !raw.trim()) return [];
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

export function normalizeHost(host: string): string {
  return host.trim().toLowerCase().replace(/^\[|\]$/g, "").replace(/\.$/, "");
}

function hasPrivateSuffix(host: string): boolean {
  return PRIVATE_SUFFIXES.some((s) => host.endsWith(s));
}

function classifyIp(ip: string): ChatEgress {
  const kind = isIP(ip);
  if (kind === 4) {
    if (LOOPBACK.check(ip, "ipv4")) return "loopback";
    if (PRIVATE.check(ip, "ipv4")) return "private";
    return "cloud";
  }
  if (kind === 6) {
    if (LOOPBACK.check(ip, "ipv6")) return "loopback";
    if (PRIVATE.check(ip, "ipv6")) return "private";
    return "cloud";
  }
  return "cloud";
}

/** Sync: IP literals, loopback names, private suffixes, extra host allowlist. No DNS. */
export function classifyHostname(host: string, extraHosts: readonly string[] = []): ChatEgress {
  const h = normalizeHost(host);
  if (!h) return "cloud";
  if (extraHosts.some((e) => normalizeHost(e) === h)) return "private";
  if (h === "localhost" || h === "localhost.localdomain") return "loopback";
  if (isIP(h)) return classifyIp(h);
  if (hasPrivateSuffix(h)) return "private";
  return "cloud";
}

export function classifyUrl(u: URL, extraHosts: readonly string[] = []): ChatEgress {
  if (u.protocol !== "http:" && u.protocol !== "https:") return "cloud";
  return classifyHostname(u.hostname, extraHosts);
}

export function isLoopbackUrl(u: URL): boolean {
  return classifyUrl(u) === "loopback";
}

export function classifyChatBaseSync(base: string, extraHosts: readonly string[] = []): ChatEgress {
  try {
    return classifyUrl(new URL(base), extraHosts);
  } catch {
    return "cloud";
  }
}

/**
 * Same as sync, then DNS: if every A/AAAA is loopback or private, treat as private.
 * Fail closed (cloud) on timeout, NXDOMAIN, or any public address.
 */
export async function classifyChatBase(
  base: string,
  extraHosts: readonly string[] = [],
): Promise<ChatEgress> {
  const sync = classifyChatBaseSync(base, extraHosts);
  if (sync !== "cloud") return sync;
  let host = "";
  try {
    host = normalizeHost(new URL(base).hostname);
  } catch {
    return "cloud";
  }
  if (!host || isIP(host)) return "cloud";
  if (
    host.endsWith(".anthropic.com") ||
    host.endsWith(".googleapis.com") ||
    host.endsWith(".openai.azure.com") ||
    host.endsWith(".azure.com") ||
    host.endsWith(".openai.com") ||
    host.endsWith(".groq.com") ||
    host.endsWith(".openrouter.ai")
  ) {
    return "cloud";
  }
  try {
    const addrs = await Promise.race([
      dns.lookup(host, { all: true, verbatim: true }),
      new Promise<never>((_, reject) => {
        setTimeout(() => reject(new Error("dns timeout")), 1500);
      }),
    ]);
    if (!addrs.length) return "cloud";
    let sawPrivate = false;
    let sawLoopback = false;
    for (const a of addrs) {
      const e = classifyIp(a.address);
      if (e === "cloud") return "cloud";
      if (e === "private") sawPrivate = true;
      if (e === "loopback") sawLoopback = true;
    }
    if (sawPrivate) return "private";
    if (sawLoopback) return "loopback";
    return "cloud";
  } catch {
    return "cloud";
  }
}

/** Follow one same-host redirect. Refuse a hop onto the public internet. */
export async function fetchStayOnBox(url: string, init: RequestInit): Promise<Response> {
  const first = await fetch(url, { ...init, redirect: "manual" });
  if (first.status < 300 || first.status >= 400) return first;
  const loc = first.headers.get("location");
  if (!loc) {
    throw new Error("redirect without Location");
  }
  const orig = new URL(url);
  const next = new URL(loc, url);
  const sameHost =
    orig.protocol === next.protocol &&
    orig.hostname === next.hostname &&
    orig.port === next.port;
  const origEgress = classifyChatBaseSync(orig.toString());
  const nextEgress = classifyChatBaseSync(next.toString());
  const leaked =
    (origEgress === "loopback" && nextEgress !== "loopback") ||
    (origEgress === "private" && nextEgress === "cloud");
  if (!sameHost || leaked) {
    throw new Error("refusing off-box or cross-host chat redirect");
  }
  return fetch(next.toString(), { ...init, redirect: "error" });
}
