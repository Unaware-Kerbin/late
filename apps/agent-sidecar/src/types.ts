export type ChatRole = "system" | "user" | "assistant" | "tool";

export interface ToolCall {
  id: string;
  type: "function";
  function: { name: string; arguments: string };
}

export interface ChatMessage {
  role: ChatRole;
  content: string | null;
  name?: string;
  tool_call_id?: string;
  tool_calls?: ToolCall[];
}

export interface OpenAiTool {
  type: "function";
  function: {
    name: string;
    description: string;
    parameters: Record<string, unknown>;
  };
}

export type ApprovalKind = "command" | "api" | "ask";

export interface PendingApproval {
  proposalId: string;
  kind: ApprovalKind;
  title: string;
  detail: Record<string, unknown>;
  linuxUnrestricted?: boolean;
  policyAllowed?: boolean;
  policyReason?: string;
  expanded?: string;
  allowAlwaysAllow?: boolean;
}

export interface ApprovalDecision {
  proposalId: string;
  allow: boolean;
  alwaysAllow?: boolean;
  answer?: string;
}

export type SseEvent =
  | { type: "delta"; text: string }
  | { type: "tool"; name: string; status: string; detail?: unknown }
  | { type: "approval"; pending: PendingApproval }
  | { type: "ask"; proposalId: string; question: string }
  | { type: "done"; message: string }
  | { type: "error"; message: string }
  | { type: "round"; n: number; max: number };

export const MAX_ROUNDS = 50;
export const TURN_TIMEOUT_MS = 90_000;
export const DAEMON_HTTP = process.env.LATE_DAEMON_HTTP ?? "http://127.0.0.1:7420";
export const DAEMON_WS = process.env.LATE_DAEMON_WS ?? "ws://127.0.0.1:7420/ws";
export const VLLM_BASE = process.env.LATE_VLLM_BASE ?? "http://127.0.0.1:8000/v1";
export const SIDECAR_PORT = Number(process.env.LATE_SIDECAR_PORT ?? 7430);

export const SYSTEM_PROMPT = `You are Late's investigation assistant for a local network terminal.
You help operators understand devices, configs, packet captures, and API responses.

Hard rules:
- You have exactly six tools. You cannot run a shell, edit files, or open sockets yourself.
- Never ask for passwords, keys, community strings, or tokens. Credentials never appear in your context.
- Open-session scrollback (including local PTY) is already attached to this turn (redacted). Ask about open sessions by name. Do not ask the operator to paste show-run, configs, or terminal output unless that attachment is empty.
- Sensitive tokens in scrollback are already replaced with [REDACTED:kind#N]. Call read_scrollback if you need a longer tail.
- propose_command and propose_api_get never execute until the operator clicks Approve AND the vendor permit list allows it. Always-allow still re-runs the permit check. Linux has no always-allow.
- You MAY read local PTY scrollback. You may NOT propose_command or propose_api_get on local PTY or SFTP. Use SSH/serial for device commands.
- Packet captures: a findings/summary digest is attached for open pcap sessions (retransmits, DNS failures, TLS alerts, ICMP unreach, RST). Use query_pcap for filtered parses. You never see payload bytes.
- API proposals are GET-only and host-pinned. FortiManager JSON-RPC is human-only.
- If context would overflow, refuse rather than silently truncating.
- Prefer inventory names when talking about devices. Be concise and operational.`;
