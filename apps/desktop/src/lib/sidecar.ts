import { rpc } from "./rpc";
import type { ApprovalPrompt, ChatMsg, SessionInfo } from "../types";

export const SIDECAR_HTTP = "http://127.0.0.1:7430";

export type SidecarEvent =
  | { type: "delta"; text: string }
  | { type: "tool"; name: string; status: string; detail?: unknown }
  | { type: "approval"; pending: ApprovalPrompt }
  | { type: "ask"; proposalId: string; question: string }
  | { type: "done"; message: string }
  | { type: "error"; message: string }
  | { type: "round"; n: number; max: number };

export async function sidecarHealth(): Promise<boolean> {
  try {
    const r = await fetch(`${SIDECAR_HTTP}/health`, { signal: AbortSignal.timeout(1500) });
    return r.ok;
  } catch {
    return false;
  }
}

export async function sidecarModels(): Promise<{ local: string[]; cursor: string[] }> {
  const r = await fetch(`${SIDECAR_HTTP}/models`);
  if (!r.ok) throw new Error(`sidecar ${r.status}`);
  return r.json() as Promise<{ local: string[]; cursor: string[] }>;
}

export async function approve(proposalId: string, allow: boolean, extra?: { alwaysAllow?: boolean; answer?: string }) {
  const r = await fetch(`${SIDECAR_HTTP}/approve`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ proposalId, allow, alwaysAllow: extra?.alwaysAllow, answer: extra?.answer }),
  });
  if (!r.ok) throw new Error(`approve failed (${r.status})`);
}

export async function stopChat(conversationId?: string) {
  await fetch(`${SIDECAR_HTTP}/stop`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ conversationId }),
  });
}

async function attachedScrollback(): Promise<string> {
  try {
    const raw = await rpc.call<unknown>("session.list");
    const arr = Array.isArray(raw)
      ? raw
      : raw && typeof raw === "object" && Array.isArray((raw as { sessions?: unknown[] }).sessions)
        ? (raw as { sessions: unknown[] }).sessions
        : [];
    const sessions = arr
      .map((item) => {
        const s = item as Partial<SessionInfo> & Record<string, unknown>;
        return {
          id: String(s.id ?? s.sessionId ?? ""),
          name: String(s.name ?? s.label ?? s.id ?? ""),
          kind: String(s.kind ?? s.type ?? ""),
        };
      })
      .filter((s) => s.id && s.kind !== "sftp");
    if (!sessions.length) {
      return "ATTACHED LIVE SCROLLBACK: no open sessions (SSH/serial/API/pcap/local PTY).";
    }
    const chunks: string[] = [
      `Open sessions you can ask about by name: ${sessions.map((s) => s.name).join(", ")}.`,
      "ATTACHED LIVE SCROLLBACK (redacted). This is the operator terminal. Do not ask them to paste configs.",
    ];
    for (const s of sessions) {
      let text = "(empty)";
      try {
        if (s.kind === "pcap") {
          let digest: unknown;
          try {
            digest = await rpc.call<unknown>("pcap.query", {
              id: s.id,
              sessionId: s.id,
              session_id: s.id,
              q: "findings",
              query: "findings",
            });
          } catch {
            digest = await rpc.call<unknown>("pcap.query", {
              id: s.id,
              sessionId: s.id,
              session_id: s.id,
              q: "summary",
              query: "summary",
            });
          }
          text = `PCAP FINDINGS (no payload bytes)\n${JSON.stringify(digest, null, 2)}`;
        } else {
          const sb = await rpc.call<{ text?: string }>("session.scrollback", {
            id: s.id,
            sessionId: s.id,
            session_id: s.id,
            redacted: true,
          });
          const t = (sb?.text ?? "").trim();
          if (t) text = t.length > 12000 ? t.slice(-12000) : t;
        }
      } catch (err) {
        text = `(unavailable: ${err instanceof Error ? err.message : String(err)})`;
      }
      chunks.push(`\n### ${s.name} id=${s.id} kind=${s.kind}\n\`\`\`\n${text}\n\`\`\``);
    }
    return chunks.join("\n");
  } catch (err) {
    return `ATTACHED LIVE SCROLLBACK unavailable: ${err instanceof Error ? err.message : String(err)}`;
  }
}

export async function streamChat(opts: {
  messages: ChatMsg[];
  backend: "local" | "cursor";
  model?: string;
  conversationId: string;
  onEvent: (e: SidecarEvent) => void;
  signal: AbortSignal;
}): Promise<void> {
  const r = await fetch(`${SIDECAR_HTTP}/chat`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      messages: await (async () => {
        const mapped = opts.messages.map((m) => ({ role: m.role, content: m.content }));
        const live = await attachedScrollback();
        const lastUser = [...mapped].reverse().find((m) => m.role === "user");
        if (lastUser) lastUser.content = `${lastUser.content ?? ""}\n\n${live}`;
        else mapped.push({ role: "user", content: live });
        return mapped;
      })(),
      backend: opts.backend,
      model: opts.model,
      conversationId: opts.conversationId,
    }),
    signal: opts.signal,
  });
  if (!r.ok || !r.body) throw new Error(`sidecar chat ${r.status}: ${await r.text()}`);
  const reader = r.body.getReader();
  const dec = new TextDecoder();
  let buf = "";
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    buf += dec.decode(value, { stream: true });
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
  }
}
