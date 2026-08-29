import { lateAuthHeaders, lateLocalToken } from "./localToken";
import type { ApprovalPrompt, ChatBackend, ChatMsg } from "../types";

export const SIDECAR_HTTP = "http://127.0.0.1:7430";

async function sidecarHeaders(base?: Record<string, string>): Promise<Record<string, string>> {
  await lateLocalToken();
  return lateAuthHeaders(base);
}

export type SidecarEvent =
  | { type: "delta"; text: string }
  | {
      type: "speaker";
      speaker: string;
      label: string;
      nickname?: string;
      logoUrl?: string;
      logoDataUrl?: string;
      content: string;
      skipped?: boolean;
    }
  | { type: "tool"; name: string; status: string; detail?: unknown }
  | { type: "approval"; pending: ApprovalPrompt }
  | { type: "ask"; proposalId: string; question: string }
  | { type: "done"; message: string }
  | { type: "error"; message: string }
  | { type: "round"; n: number; max: number }
  | { type: "heartbeat"; message: string };

export async function sidecarHealth(): Promise<boolean> {
  try {
    const r = await fetch(`${SIDECAR_HTTP}/health`, { signal: AbortSignal.timeout(1500) });
    return r.ok;
  } catch {
    return false;
  }
}

export type ChatEgress = "loopback" | "private" | "cloud";

export type ChatTargetMeta = { base: string; egress: ChatEgress };

export type SidecarModels = {
  local: string[];
  cursor: string[];
  ollama: string[];
  ollamaOk?: boolean;
  ollamaMessage?: string;
  llamacpp: string[];
  llamacppOk?: boolean;
  llamacppMessage?: string;
  anthropic?: string[];
  gemini?: string[];
  azure?: string[];
  backends: string[];
  targets?: {
    local?: ChatTargetMeta;
    ollama?: ChatTargetMeta;
    llamacpp?: ChatTargetMeta;
    anthropic?: ChatTargetMeta;
    gemini?: ChatTargetMeta;
    azure?: ChatTargetMeta;
    mcp?: ChatTargetMeta;
  };
  mcp?: {
    ok: boolean;
    enabled: boolean;
    tools: string[];
    message: string;
    agents?: string[];
  };
};

export async function sidecarModels(): Promise<SidecarModels> {
  const r = await fetch(`${SIDECAR_HTTP}/models`, { headers: await sidecarHeaders() });
  if (!r.ok) throw new Error(`sidecar ${r.status}`);
  const body = (await r.json()) as Partial<SidecarModels>;
  return {
    local: body.local ?? [],
    cursor: body.cursor ?? ["composer-2.5", "auto"],
    ollama: body.ollama ?? [],
    ollamaOk: body.ollamaOk,
    ollamaMessage: body.ollamaMessage,
    llamacpp: body.llamacpp ?? [],
    llamacppOk: body.llamacppOk,
    llamacppMessage: body.llamacppMessage,
    anthropic: body.anthropic ?? [],
    gemini: body.gemini ?? [],
    azure: body.azure ?? [],
    backends: body.backends ?? ["local", "llamacpp", "ollama", "anthropic", "gemini", "azure", "cursor", "mcp"],
    targets: body.targets,
    mcp: body.mcp ? { ...body.mcp, agents: ["orchestrator"] } : body.mcp,
  };
}

export async function sidecarProbe(
  kind: "vllm" | "ollama" | "llamacpp" | "anthropic" | "gemini" | "azure" | "mcp",
  base = "",
): Promise<{
  ok: boolean;
  base?: string;
  egress?: ChatEgress;
  models?: string[];
  tools?: string[];
  enabled?: boolean;
  message: string;
}> {
  const r = await fetch(`${SIDECAR_HTTP}/probe`, {
    method: "POST",
    headers: await sidecarHeaders({ "Content-Type": "application/json" }),
    body: JSON.stringify({ kind, ...(base ? { base } : {}) }),
    signal: AbortSignal.timeout(kind === "mcp" ? 20_000 : 8000),
  });
  if (!r.ok) throw new Error(`sidecar probe ${r.status}`);
  return (await r.json()) as {
    ok: boolean;
    base?: string;
    egress?: ChatEgress;
    models?: string[];
    tools?: string[];
    enabled?: boolean;
    message: string;
  };
}

export async function sidecarGrantDir(path: string): Promise<{
  ok: boolean;
  cwd?: string;
  denied?: boolean;
  error?: string;
  note?: string;
}> {
  const r = await fetch(`${SIDECAR_HTTP}/mcp/grant-dir`, {
    method: "POST",
    headers: await sidecarHeaders({ "Content-Type": "application/json" }),
    body: JSON.stringify({ path }),
  });
  const body = (await r.json().catch(() => ({}))) as {
    ok?: boolean;
    cwd?: string;
    denied?: boolean;
    error?: string;
    note?: string;
  };
  if (!r.ok && !body.error) {
    throw new Error(`sidecar grant-dir ${r.status}`);
  }
  return {
    ok: Boolean(body.ok),
    cwd: body.cwd,
    denied: body.denied,
    error: body.error,
    note: body.note,
  };
}

export async function sidecarPending(): Promise<ApprovalPrompt[]> {
  try {
    const r = await fetch(`${SIDECAR_HTTP}/pending`, {
      headers: await sidecarHeaders(),
      signal: AbortSignal.timeout(2000),
    });
    if (!r.ok) return [];
    const body = (await r.json()) as { pending?: ApprovalPrompt[] };
    return Array.isArray(body.pending) ? body.pending : [];
  } catch {
    return [];
  }
}

export async function approve(proposalId: string, allow: boolean, extra?: { alwaysAllow?: boolean; answer?: string }) {
  const r = await fetch(`${SIDECAR_HTTP}/approve`, {
    method: "POST",
    headers: await sidecarHeaders({ "Content-Type": "application/json" }),
    body: JSON.stringify({ proposalId, allow, alwaysAllow: extra?.alwaysAllow, answer: extra?.answer }),
  });
  if (!r.ok) throw new Error(`approve failed (${r.status})`);
}

export async function stopChat(conversationId?: string) {
  await fetch(`${SIDECAR_HTTP}/stop`, {
    method: "POST",
    headers: await sidecarHeaders({ "Content-Type": "application/json" }),
    body: JSON.stringify({ conversationId }),
  });
}

export async function streamChat(opts: {
  messages: ChatMsg[];
  backend: ChatBackend;
  model?: string;
  conversationId: string;
  onEvent: (e: SidecarEvent) => void;
  signal: AbortSignal;
}): Promise<void> {
  const r = await fetch(`${SIDECAR_HTTP}/chat`, {
    method: "POST",
    headers: await sidecarHeaders({ "Content-Type": "application/json" }),
    body: JSON.stringify({
      messages: opts.messages.map((m) => ({ role: m.role, content: m.content })),
      backend: opts.backend,
      model: opts.model,
      conversationId: opts.conversationId,
    }),
    signal: opts.signal,
  });
  if (r.status === 401) {
    throw new Error("sidecar rejected chat (401). Restart the sidecar after the daemon token change.");
  }
  if (!r.ok || !r.body) throw new Error(`sidecar chat ${r.status}: ${await r.text()}`);
  const reader = r.body.getReader();
  const dec = new TextDecoder();
  let buf = "";
  const flush = (chunk: string) => {
    buf += chunk;
    const parts = buf.split("\n\n");
    buf = parts.pop() ?? "";
    for (const part of parts) {
      const dataLine = part.split("\n").find((l) => l.startsWith("data:"));
      if (!dataLine) continue;
      try {
        opts.onEvent(JSON.parse(dataLine.slice(5).trim()) as SidecarEvent);
      } catch {
        /* ignore malformed */
      }
    }
  };
  while (true) {
    const { done, value } = await reader.read();
    if (value) flush(dec.decode(value, { stream: !done }));
    if (done) {
      if (buf.trim()) flush("\n\n");
      break;
    }
  }
}
