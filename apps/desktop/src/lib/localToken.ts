let cached = "";

type LateRuntime = { token?: () => Promise<string> };

export async function lateLocalToken(): Promise<string> {
  if (cached) return cached;
  const runtime = (window as unknown as { lateRuntime?: LateRuntime }).lateRuntime;
  if (typeof runtime?.token === "function") {
    try {
      const t = ((await runtime.token()) ?? "").trim();
      if (t) {
        cached = t;
        return t;
      }
    } catch {
      /* packaged token read failed; try Vite endpoint in dev */
    }
  }
  try {
    const r = await fetch("/__late_token", { cache: "no-store" });
    if (!r.ok) return "";
    const t = (await r.text()).trim();
    if (t) cached = t;
    return t;
  } catch {
    return "";
  }
}

export function lateAuthHeaders(base?: Record<string, string>): Record<string, string> {
  const headers = { ...(base ?? {}) };
  if (cached) headers["X-Late-Token"] = cached;
  return headers;
}
