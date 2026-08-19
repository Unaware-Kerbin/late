import { daemon } from "./daemon.js";
import { isAlwaysAllowed, listPending, rememberAlways, requestApproval } from "./approvals.js";
import type { ApprovalDecision, OpenAiTool, PendingApproval, SseEvent } from "./types.js";

export type ToolCtx = {
  conversationId: string;
  emit: (e: SseEvent) => void;
  signal: AbortSignal;
};

/** Six operator-gated tools only. Never add SFTP — file transfer is UI-only. */
export const OPENAI_TOOLS: OpenAiTool[] = [
  {
    type: "function",
    function: {
      name: "list_open_sessions",
      description: "List currently open sessions. Returns names, kinds, vendors, and ids only — never credentials.",
      parameters: { type: "object", properties: {}, additionalProperties: false },
    },
  },
  {
    type: "function",
    function: {
      name: "read_scrollback",
      description: "Read redacted scrollback for a session. Sensitive tokens are already replaced with [REDACTED:kind#N].",
      parameters: {
        type: "object",
        properties: { session_id: { type: "string" } },
        required: ["session_id"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "propose_command",
      description:
        "Propose a CLI command on a session. The vendor permit list runs first, then the operator must click Approve. This tool never executes by itself.",
      parameters: {
        type: "object",
        properties: {
          session_id: { type: "string" },
          command: { type: "string" },
        },
        required: ["session_id", "command"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "propose_api_get",
      description:
        "Propose a GET request to a host-pinned API session. FortiManager JSON-RPC is rejected. Operator must click Approve.",
      parameters: {
        type: "object",
        properties: {
          session_id: { type: "string" },
          path: { type: "string", description: "Path or full URL on the pinned host" },
        },
        required: ["session_id", "path"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "query_pcap",
      description:
        "Parse an open pcap session. query 'summary' or 'findings' for header-derived analysis (retransmits, zero-window, DNS rcode failures, TLS alerts, ICMP unreach, TCP RST). Otherwise a substring or display-filter-like text (dns, tcp, an IP, ip.src==1.2.3.4). Payload bytes are never returned.",
      parameters: {
        type: "object",
        properties: {
          session_id: { type: "string" },
          query: { type: "string" },
        },
        required: ["session_id", "query"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "ask_user",
      description: "Ask the operator a clarifying question and wait for their reply.",
      parameters: {
        type: "object",
        properties: { question: { type: "string" } },
        required: ["question"],
        additionalProperties: false,
      },
    },
  },
];

export async function snapshotSessions(): Promise<
  { id: string; name: string; kind: string; vendor?: string }[]
> {
  const raw = await daemon.call<unknown>("session.list");
  const arr = Array.isArray(raw)
    ? raw
    : raw && typeof raw === "object" && Array.isArray((raw as { sessions?: unknown[] }).sessions)
      ? (raw as { sessions: unknown[] }).sessions
      : [];
  return arr
    .map((item) => {
      const s = item as Record<string, unknown>;
      const id = String(s.id ?? s.sessionId ?? s.session_id ?? "");
      const name = String(s.name ?? s.label ?? id);
      const kind = String(s.kind ?? s.type ?? "ssh");
      const vendor = s.vendor != null ? String(s.vendor) : undefined;
      return { id, name, kind, vendor };
    })
    .filter((s) => s.id && s.kind !== "sftp");
}

const MAX_SCROLLBACK_CHARS = 12_000;
const MAX_CONTEXT_CHARS = 48_000;

/** Redacted live terminals for the agent — Cursor has no working tool loop unless this is inlined. */
export async function liveSessionContext(): Promise<string> {
  const sessions = await snapshotSessions();
  if (!sessions.length) {
    return "No sessions are open (SSH, serial, API, pcap, or local PTY). Ask the operator to connect a device. Do not invent session ids. Do not ask them to paste configs.";
  }
  const names = sessions
    .map((s) => `${s.name} (${s.kind}${s.vendor ? `, ${s.vendor}` : ""})`)
    .join(", ");
  const parts: string[] = [
    `Open sessions you can ask about by name: ${names}.`,
    "ATTACHED LIVE SCROLLBACK (already redacted). This is the operator's terminal. Do not ask them to paste configs. Local PTY is read-only (no propose_command). Use these exact ids with read_scrollback if you need a longer tail.",
  ];
  let used = parts.join("\n").length;
  for (const s of sessions) {
    const head = `\n### ${s.name}  id=${s.id}  kind=${s.kind}${s.vendor ? `  vendor=${s.vendor}` : ""}`;
    let body = "(no scrollback yet)";
    try {
      if (s.kind === "pcap") {
        let digest: unknown;
        try {
          digest = await daemon.call<unknown>("pcap.query", {
            id: s.id,
            sessionId: s.id,
            session_id: s.id,
            q: "findings",
            query: "findings",
          });
        } catch {
          digest = await daemon.call<unknown>("pcap.query", {
            id: s.id,
            sessionId: s.id,
            session_id: s.id,
            q: "summary",
            query: "summary",
          });
        }
        body = `PCAP FINDINGS (no payload bytes)\n${JSON.stringify(digest, null, 2)}`;
      } else {
        const raw = await daemon.call<{ text?: string }>("session.scrollback", {
          id: s.id,
          sessionId: s.id,
          session_id: s.id,
          redacted: true,
        });
        const text = (raw?.text ?? "").replace(/\x1b\[[0-9;]*[A-Za-z]/g, "").trim();
        if (text) {
          body = text.length > MAX_SCROLLBACK_CHARS ? text.slice(-MAX_SCROLLBACK_CHARS) : text;
        }
      }
    } catch (err) {
      body = `(unavailable: ${err instanceof Error ? err.message : String(err)})`;
    }
    const chunk = `${head}\n\`\`\`\n${body}\n\`\`\``;
    if (used + chunk.length > MAX_CONTEXT_CHARS) {
      parts.push("\n(further session scrollback omitted to stay under the context cap; call read_scrollback.)");
      break;
    }
    parts.push(chunk);
    used += chunk.length;
  }
  return parts.join("\n");
}

function str(args: Record<string, unknown>, key: string): string {
  const camel = key.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase());
  const v = args[key] ?? args[camel];
  if (typeof v !== "string" || !v.trim()) throw new Error(`missing ${key}`);
  return v;
}

function waitAbort(signal: AbortSignal): Promise<never> {
  return new Promise((_, reject) => {
    if (signal.aborted) {
      reject(new Error("stopped"));
      return;
    }
    signal.addEventListener("abort", () => reject(new Error("stopped")), { once: true });
  });
}

function latestPending(pred: (p: PendingApproval) => boolean): PendingApproval | undefined {
  return [...listPending()].reverse().find(pred);
}

export async function executeTool(name: string, rawArgs: string, ctx: ToolCtx): Promise<string> {
  let args: Record<string, unknown> = {};
  try {
    args = rawArgs ? (JSON.parse(rawArgs) as Record<string, unknown>) : {};
  } catch {
    return JSON.stringify({ error: "invalid tool arguments JSON" });
  }
  ctx.emit({ type: "tool", name, status: "start", detail: args });
  try {
    const out = await dispatch(name, args, ctx);
    const obj = typeof out === "object" && out !== null ? (out as Record<string, unknown>) : null;
    const denied = obj?.policyAllowed === false;
    ctx.emit({
      type: "tool",
      name,
      status: denied ? "denied" : "ok",
      detail: denied ? obj?.reason : undefined,
    });
    return typeof out === "string" ? out : JSON.stringify(out);
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    ctx.emit({ type: "tool", name, status: "error", detail: message });
    return JSON.stringify({ error: message });
  }
}

async function dispatch(name: string, args: Record<string, unknown>, ctx: ToolCtx): Promise<unknown> {
  switch (name) {
    case "list_open_sessions":
      return snapshotSessions();
    case "read_scrollback": {
      const sessionId = str(args, "session_id");
      return daemon.call("session.scrollback", { sessionId, session_id: sessionId, redacted: true });
    }
    case "query_pcap": {
      const sessionId = str(args, "session_id");
      const query = str(args, "query");
      return daemon.call("pcap.query", { sessionId, session_id: sessionId, query });
    }
    case "ask_user":
      return askUser(str(args, "question"), ctx);
    case "propose_command":
      return proposeCommand(str(args, "session_id"), str(args, "command"), ctx);
    case "propose_api_get":
      return proposeApi(str(args, "session_id"), str(args, "path"), ctx);
    default:
      return { error: `unknown tool ${name} — only the six Late tools exist` };
  }
}

async function awaitGate(
  ctx: ToolCtx,
  pending: Omit<PendingApproval, "proposalId">,
  match: (p: PendingApproval) => boolean,
): Promise<ApprovalDecision> {
  const req = requestApproval(pending);
  const live = latestPending(match);
  ctx.emit({ type: "approval", pending: live ?? { ...pending, proposalId: "pending" } });
  return Promise.race([req, waitAbort(ctx.signal)]);
}

async function askUser(question: string, ctx: ToolCtx) {
  const pending: Omit<PendingApproval, "proposalId"> = {
    kind: "ask",
    title: "Agent question",
    detail: { question },
  };
  const req = requestApproval(pending);
  const live = latestPending((p) => p.kind === "ask" && p.detail.question === question);
  if (live) {
    ctx.emit({ type: "approval", pending: live });
    ctx.emit({ type: "ask", proposalId: live.proposalId, question });
  }
  const decision = await Promise.race([req, waitAbort(ctx.signal)]);
  if (!decision.allow) return { answered: false, note: "operator declined to answer" };
  return { answered: true, answer: decision.answer ?? "" };
}

async function proposeCommand(sessionId: string, command: string, ctx: ToolCtx) {
  const sessions = await snapshotSessions();
  const sess = sessions.find((s) => s.id === sessionId);
  if (sess && (sess.kind === "local" || sess.kind === "sftp")) {
    return {
      executed: false,
      reason: "propose_command is not allowed on local PTY or SFTP; open an SSH/serial session to the device instead",
    };
  }
  const policy = (await daemon.call("policy.check", {
    sessionId,
    session_id: sessionId,
    command,
  })) as {
    allowed?: boolean;
    reason?: string;
    expanded?: string;
    linuxUnrestricted?: boolean;
    linux_unrestricted?: boolean;
    allowAlwaysAllow?: boolean;
    allow_always_allow?: boolean;
  };
  const linux = Boolean(policy.linuxUnrestricted ?? policy.linux_unrestricted);
  const allowed = policy.allowed !== false;
  const expanded = policy.expanded ?? command;
  const fingerprint = `${sessionId}::${expanded}`;
  const alwaysOk =
    allowed &&
    !linux &&
    (policy.allowAlwaysAllow ?? policy.allow_always_allow) !== false;

  if (!allowed) {
    ctx.emit({ type: "tool", name: "propose_command", status: "denied", detail: policy.reason });
    await awaitGate(
      ctx,
      {
        kind: "command",
        title: "Command blocked by permit list",
        detail: { sessionId, command, expanded },
        linuxUnrestricted: linux,
        policyAllowed: false,
        policyReason: policy.reason ?? "permit list denied",
        expanded,
        allowAlwaysAllow: false,
      },
      (p) => p.kind === "command" && p.detail.command === command,
    );
    return {
      executed: false,
      policyAllowed: false,
      reason: policy.reason ?? "permit list denied",
      expanded,
    };
  }

  let allow = isAlwaysAllowed(ctx.conversationId, "command", fingerprint, linux);
  if (allow) {
    ctx.emit({ type: "tool", name: "propose_command", status: "always-allow-reuse" });
  } else {
    const decision = await awaitGate(
      ctx,
      {
        kind: "command",
        title: "Approve command",
        detail: { sessionId, command, expanded },
        linuxUnrestricted: linux,
        policyAllowed: true,
        policyReason: policy.reason,
        expanded,
        allowAlwaysAllow: alwaysOk,
      },
      (p) => p.kind === "command" && p.detail.command === command,
    );
    allow = decision.allow;
    if (decision.alwaysAllow && alwaysOk) rememberAlways(ctx.conversationId, "command", fingerprint, linux);
  }

  if (!allow) return { executed: false, reason: "operator denied" };

  const payload = command.endsWith("\n") ? command : `${command}\n`;
  const data = Buffer.from(payload, "utf8").toString("base64");
  await daemon.call("session.input", { sessionId, session_id: sessionId, data });
  return { executed: true, expanded, note: "command written to session after dual gate" };
}

async function proposeApi(sessionId: string, path: string, ctx: ToolCtx) {
  if (/fortimanager|fortigate|jsonrpc|json-rpc/i.test(path)) {
    return { executed: false, reason: "FortiManager JSON-RPC is human-only" };
  }
  const fingerprint = `${sessionId}::GET::${path}`;
  let allow = isAlwaysAllowed(ctx.conversationId, "api", fingerprint, false);
  if (!allow) {
    const decision = await awaitGate(
      ctx,
      {
        kind: "api",
        title: "Approve API GET",
        detail: { sessionId, method: "GET", path },
        policyAllowed: true,
      },
      (p) => p.kind === "api" && p.detail.path === path,
    );
    allow = decision.allow;
    if (decision.alwaysAllow) rememberAlways(ctx.conversationId, "api", fingerprint, false);
  }
  if (!allow) return { executed: false, reason: "operator denied" };
  const result = await daemon.call("api.request", {
    sessionId,
    session_id: sessionId,
    method: "GET",
    url: path,
    agent: true,
  });
  return { executed: true, result };
}

export function cursorToolDefs(ctxFactory: () => ToolCtx) {
  return Object.fromEntries(
    OPENAI_TOOLS.map((t) => {
      const name = t.function.name;
      return [
        name,
        {
          description: t.function.description,
          inputSchema: t.function.parameters,
          async execute(args: Record<string, unknown>) {
            return executeTool(name, JSON.stringify(args ?? {}), ctxFactory());
          },
        },
      ];
    }),
  );
}
