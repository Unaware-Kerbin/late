import { timingSafeEqual } from "node:crypto";
import { appendFileSync, mkdirSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { IncomingMessage } from "node:http";

const UI_PORTS = new Set(["5173", "4173", "1420"]);

export function lateConfigDir(): string {
  const home = homedir();
  if (process.platform === "win32") {
    return join(process.env.APPDATA || join(home, "AppData", "Roaming"), "late");
  }
  if (process.platform === "darwin") {
    return join(home, "Library", "Application Support", "late");
  }
  return join(process.env.XDG_CONFIG_HOME || join(home, ".config"), "late");
}

export function lateDataDir(): string {
  const home = homedir();
  if (process.platform === "win32") {
    return join(process.env.APPDATA || join(home, "AppData", "Roaming"), "late");
  }
  if (process.platform === "darwin") {
    return join(home, "Library", "Application Support", "late");
  }
  return join(process.env.XDG_DATA_HOME || join(home, ".local/share"), "late");
}

export function auditEvent(event: string, fields: Record<string, unknown> = {}): void {
  try {
    const dir = lateDataDir();
    mkdirSync(dir, { recursive: true, mode: 0o700 });
    const user = process.env.USER || process.env.USERNAME || "unknown";
    appendFileSync(
      join(dir, "audit.jsonl"),
      `${JSON.stringify({ ts: new Date().toISOString(), event, user, ...fields })}\n`,
      { mode: 0o600 },
    );
  } catch {
    /* never fail the request on audit */
  }
}

export function readLocalToken(): string {
  const env = (process.env.LATE_RPC_TOKEN ?? "").trim();
  if (env) return env;
  try {
    return readFileSync(join(lateConfigDir(), "sidecar.token"), "utf8").trim();
  } catch {
    return "";
  }
}

/** Keep in sync with crates/late-core/src/origin.rs */

/** CORS only — never used as auth by itself. Lets the Electron file:// UI read responses. */
export function corsAllowOrigin(origin: string | undefined): string | null {
  if (!origin) return null;
  if (isAllowedOrigin(origin)) return origin;
  const lower = origin.trim().toLowerCase();
  if (lower === "file://" || lower === "null") return origin;
  if (
    lower === "tauri://localhost" ||
    lower === "https://tauri.localhost" ||
    lower === "http://tauri.localhost"
  ) {
    return origin;
  }
  return null;
}

export function isAllowedOrigin(origin: string | undefined): boolean {
  if (!origin) return false;
  const o = origin.trim();
  if (!o || o.toLowerCase() === "null") return false;
  const lower = o.toLowerCase();
  if (
    lower === "tauri://localhost" ||
    lower === "https://tauri.localhost" ||
    lower === "http://tauri.localhost"
  ) {
    return true;
  }
  let u: URL;
  try {
    u = new URL(o);
  } catch {
    return false;
  }
  if (u.username || u.password) return false;
  if (u.protocol !== "http:" && u.protocol !== "https:") return false;
  const host = u.hostname.replace(/^\[|\]$/g, "");
  const loopback = host === "127.0.0.1" || host === "localhost" || host === "::1";
  return loopback && UI_PORTS.has(u.port);
}

export function isLoopbackHostHeader(host: string | undefined): boolean {
  if (!host) return false;
  const h = host.trim().toLowerCase();
  if (h.startsWith("[")) {
    const end = h.indexOf("]");
    if (end < 1) return false;
    return h.slice(1, end) === "::1";
  }
  const name = h.includes(":") ? h.slice(0, h.lastIndexOf(":")) : h;
  return name === "127.0.0.1" || name === "localhost" || name === "::1";
}

export function tokenEquals(presented: string, stored: string): boolean {
  if (!presented || !stored) return false;
  const a = Buffer.from(presented);
  const b = Buffer.from(stored);
  if (a.length !== b.length) return false;
  return timingSafeEqual(a, b);
}

export function presentedToken(req: IncomingMessage, url: URL): string {
  const header = req.headers["x-late-token"];
  if (typeof header === "string" && header.trim()) return header.trim();
  const sidecar = req.headers["x-late-sidecar-token"];
  if (typeof sidecar === "string" && sidecar.trim()) return sidecar.trim();
  const auth = req.headers.authorization;
  if (typeof auth === "string") {
    const m = /^Bearer\s+(.+)$/i.exec(auth.trim());
    if (m?.[1]) return m[1].trim();
  }
  return (url.searchParams.get("token") ?? "").trim();
}

/** Token is authentication. Origin is CORS only. */
export function authorizeLocal(req: IncomingMessage, url: URL): boolean {
  const stored = readLocalToken();
  return tokenEquals(presentedToken(req, url), stored);
}
