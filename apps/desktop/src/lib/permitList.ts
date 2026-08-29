import { VENDORS, type Vendor } from "../types";

/** Device-CLI permit list on your computer — not MCP grant-folder. */
export type PermitPolicyView = {
  vendor: Vendor;
  unrestricted: boolean;
  allowAlwaysAllow: boolean;
  allow: string[];
  builtinAllow: string[];
  deny: string[];
  denySubstrings: string[];
  path: string;
};

export const DANGEROUS_PERMIT_VERBS = [
  "reload",
  "reboot",
  "erase",
  "write erase",
  "boot",
  "start-shell",
  "shell",
  "factory",
  "copy",
  "bash",
  "python",
  "tclsh",
  "format",
  "guestshell",
  "shutdown",
] as const;

export function isDangerousPermitVerb(token: string): boolean {
  const t = token.trim().toLowerCase();
  const first = t.split(/\s+/)[0] ?? "";
  return DANGEROUS_PERMIT_VERBS.some((d) => t === d || first === d || t.startsWith(`${d} `));
}

export function normalizePermitToken(raw: string): string | null {
  const t = raw.trim();
  if (!t) return null;
  if (/[\n\r|]/.test(t) || t.length > 64) return null;
  for (let i = 0; i < t.length; i++) {
    const c = t.charCodeAt(i);
    if (c < 32) return null;
  }
  return t;
}

export function addPermitToken(allow: string[], raw: string): { allow: string[]; token: string } | { error: string } {
  const token = normalizePermitToken(raw);
  if (!token) return { error: "Type a single verb (no newlines or pipes)." };
  if (allow.some((a) => a.toLowerCase() === token.toLowerCase())) {
    return { allow: [...allow], token };
  }
  return { allow: [...allow, token], token };
}

export function removePermitToken(allow: string[], token: string): string[] {
  const key = token.toLowerCase();
  return allow.filter((a) => a.toLowerCase() !== key);
}

function strList(raw: unknown, ...keys: string[]): string[] {
  if (!raw || typeof raw !== "object") return [];
  const r = raw as Record<string, unknown>;
  for (const k of keys) {
    const v = r[k];
    if (Array.isArray(v)) return v.filter((x): x is string => typeof x === "string");
  }
  return [];
}

function str(raw: unknown, ...keys: string[]): string {
  if (!raw || typeof raw !== "object") return "";
  const r = raw as Record<string, unknown>;
  for (const k of keys) {
    const v = r[k];
    if (typeof v === "string") return v;
  }
  return "";
}

function flag(raw: unknown, ...keys: string[]): boolean {
  if (!raw || typeof raw !== "object") return false;
  const r = raw as Record<string, unknown>;
  for (const k of keys) {
    const v = r[k];
    if (typeof v === "boolean") return v;
  }
  return false;
}

function asVendor(s: string): Vendor {
  return (VENDORS as string[]).includes(s) ? (s as Vendor) : "generic";
}

export function coercePermitPolicy(raw: unknown): PermitPolicyView | null {
  if (!raw || typeof raw !== "object") return null;
  const vendor = asVendor(str(raw, "vendor"));
  return {
    vendor,
    unrestricted: flag(raw, "unrestricted") || vendor === "linux",
    allowAlwaysAllow: vendor === "linux" ? false : flag(raw, "allowAlwaysAllow", "allow_always_allow"),
    allow: strList(raw, "allow"),
    builtinAllow: strList(raw, "builtinAllow", "builtin_allow"),
    deny: strList(raw, "deny"),
    denySubstrings: strList(raw, "denySubstrings", "deny_substrings"),
    path: str(raw, "path"),
  };
}

export function coercePermitPolicyList(raw: unknown): PermitPolicyView[] {
  const arr = Array.isArray(raw)
    ? raw
    : raw && typeof raw === "object" && Array.isArray((raw as { policies?: unknown }).policies)
      ? (raw as { policies: unknown[] }).policies
      : [];
  return arr.map(coercePermitPolicy).filter((p): p is PermitPolicyView => p !== null);
}
