import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { DAEMON_HTTP } from "./types.js";

export type ProviderMap = Record<string, string>;

let cache: { at: number; keys: ProviderMap } | null = null;
const CACHE_MS = 5_000;

async function sidecarToken(): Promise<string> {
  const path = join(homedir(), ".config/late/sidecar.token");
  const raw = await readFile(path, "utf8");
  return raw.trim();
}

export async function loadProviderKeys(): Promise<ProviderMap> {
  if (cache && Date.now() - cache.at < CACHE_MS) return cache.keys;
  const token = await sidecarToken().catch(() => "");
  if (!token) return {};
  const r = await fetch(`${DAEMON_HTTP}/internal/provider-keys`, {
    headers: { "x-late-sidecar-token": token },
    signal: AbortSignal.timeout(2000),
  });
  if (!r.ok) return {};
  const json = (await r.json()) as ProviderMap;
  const keys: ProviderMap = {};
  for (const [k, v] of Object.entries(json)) {
    if (typeof v === "string" && v.length > 0) keys[k] = v;
  }
  cache = { at: Date.now(), keys };
  return keys;
}

export function forgetProviderKeys() {
  cache = null;
}
