export type DeviceKind = "ssh" | "serial" | "local" | "api";
export type SessionKind = "ssh" | "serial" | "local" | "sftp" | "pcap" | "api";
export type Vendor =
  | "junos"
  | "cisco_ios"
  | "cisco_ios_xe"
  | "cisco_nxos"
  | "arista_eos"
  | "panos"
  | "linux"
  | "fortios"
  | "routeros"
  | "aos_cx"
  | "generic";

export const VENDORS: Vendor[] = [
  "junos",
  "cisco_ios",
  "cisco_ios_xe",
  "cisco_nxos",
  "arista_eos",
  "panos",
  "linux",
  "fortios",
  "routeros",
  "aos_cx",
  "generic",
];

export interface Device {
  id: string;
  name: string;
  kind: DeviceKind;
  vendor: Vendor;
  host?: string | null;
  port?: number | null;
  serial_path?: string | null;
  baud?: number | null;
  api_base_url?: string | null;
  api_controller?: string | null;
  auth_profile_id?: string | null;
  folder?: string | null;
  tags: string[];
  accent?: string | null;
  syntax_highlight: boolean;
  quick_copy: boolean;
  legacy_ssh: boolean;
  jump_host?: string | null;
  shell?: string | null;
  notes?: string | null;
}

export interface AuthProfile {
  id: string;
  name: string;
  username: string;
  key_path?: string | null;
  use_agent: boolean;
  has_password: boolean;
}

export interface Inventory {
  devices: Device[];
  folders: string[];
}

export interface SessionInfo {
  id: string;
  device_id?: string | null;
  name: string;
  kind: SessionKind;
  vendor: Vendor;
  connected: boolean;
  created_at: string;
  accent?: string | null;
}

export interface PolicyDecision {
  allowed: boolean;
  reason: string;
  expanded: string;
  linux_unrestricted: boolean;
  allow_always_allow?: boolean;
}

export interface SftpEntry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
}

export interface PacketSummary {
  index: number;
  timestamp: string;
  src: string;
  dst: string;
  protocol: string;
  length: number;
  info: string;
  fields: Record<string, string>;
}

export interface PcapFinding {
  id: string;
  kind: string;
  summary: string;
  packet_indexes: number[];
  evidence: string;
}

export interface CommandCollection {
  id: string;
  name: string;
  vendor?: Vendor | null;
  commands: string[];
}

export interface CaptureRecord {
  id: string;
  name: string;
  device_id?: string | null;
  command: string;
  output: string;
  created_at: string;
}

export interface DiffLine {
  tag: string;
  text: string;
}

export interface AppSettings {
  bind: string;
  vllm_base_url: string;
  vllm_model: string;
  cursor_model: string;
  default_backend: string;
  scrollback_lines: number;
  turn_timeout_secs: number;
  max_agent_rounds: number;
}

export type SplitDir = "right" | "down";

export type SplitNode =
  | { type: "leaf"; paneId: string }
  | { type: "split"; dir: SplitDir; ratio: number; a: SplitNode; b: SplitNode };

export interface PaneState {
  id: string;
  kind: SessionKind | "empty";
  session?: SessionInfo;
  deviceId?: string;
  disconnected: boolean;
  disconnectReason?: string;
  searchOpen: boolean;
  logging?: boolean;
}

export interface TabState {
  id: string;
  title: string;
  tree: SplitNode;
}

export interface HostKeyPrompt {
  host: string;
  presented: string;
  pinned?: string;
  mismatch: boolean;
  retry: () => Promise<void>;
}

export interface ApprovalPrompt {
  proposalId: string;
  kind: "command" | "api" | "ask";
  title: string;
  detail: Record<string, unknown>;
  linuxUnrestricted?: boolean;
  policyAllowed?: boolean;
  policyReason?: string;
  expanded?: string;
  allowAlwaysAllow?: boolean;
}

export interface ChatMsg {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
}

export function newId(): string {
  return crypto.randomUUID();
}

function pickRaw(r: Record<string, unknown>, ...keys: string[]): unknown {
  for (const k of keys) {
    if (r[k] !== undefined && r[k] !== null) return r[k];
  }
  return undefined;
}

function pickStr(r: Record<string, unknown>, ...keys: string[]): string | null {
  const v = pickRaw(r, ...keys);
  if (typeof v === "string" && v) return v;
  return null;
}

export function coerceDeviceKind(v: unknown): DeviceKind {
  const s = String(v ?? "").toLowerCase();
  if (s === "serial" || s === "local" || s === "api" || s === "ssh") return s;
  return "ssh";
}

export function coerceDevice(raw: unknown): Device {
  const d = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  const kind = coerceDeviceKind(d.kind);
  const base = emptyDevice(kind);
  return {
    ...base,
    id: String(d.id ?? base.id),
    name: String(d.name ?? ""),
    kind,
    vendor: (String(d.vendor ?? base.vendor) as Device["vendor"]) || base.vendor,
    host: pickStr(d, "host") ?? (kind === "ssh" || kind === "api" ? String(d.host ?? "") : null),
    port: typeof d.port === "number" ? d.port : base.port,
    serial_path: pickStr(d, "serial_path", "serialPath", "path") ?? base.serial_path,
    baud: typeof d.baud === "number" ? d.baud : base.baud,
    api_base_url: pickStr(d, "api_base_url", "apiBaseUrl") ?? base.api_base_url,
    api_controller: pickStr(d, "api_controller", "apiController") ?? base.api_controller,
    auth_profile_id: pickStr(d, "auth_profile_id", "authProfileId", "auth_profile", "profile_id"),
    folder: pickStr(d, "folder"),
    tags: Array.isArray(d.tags) ? (d.tags as string[]) : [],
    accent: pickStr(d, "accent") ?? base.accent,
    syntax_highlight: Boolean(pickRaw(d, "syntax_highlight", "syntaxHighlight") ?? true),
    quick_copy: Boolean(pickRaw(d, "quick_copy", "quickCopy") ?? true),
    legacy_ssh: Boolean(pickRaw(d, "legacy_ssh", "legacySsh") ?? false),
    jump_host: pickStr(d, "jump_host", "jumpHost"),
    shell: pickStr(d, "shell") ?? base.shell,
    notes: pickStr(d, "notes"),
  };
}

export function coerceAuth(raw: unknown): AuthProfile {
  const p = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  return {
    id: String(p.id ?? ""),
    name: String(p.name ?? ""),
    username: String(pickRaw(p, "username", "user", "userName") ?? ""),
    key_path: pickStr(p, "key_path", "keyPath"),
    use_agent: Boolean(pickRaw(p, "use_agent", "useAgent") ?? false),
    has_password: Boolean(pickRaw(p, "has_password", "hasPassword") ?? false),
  };
}

export function coerceSession(raw: unknown): SessionInfo {
  const s = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  const kind = String(s.kind ?? "ssh").toLowerCase() as SessionKind;
  return {
    ...(s as unknown as SessionInfo),
    id: String(s.id ?? ""),
    device_id: pickStr(s, "device_id", "deviceId"),
    name: String(s.name ?? ""),
    kind,
  };
}

export function coerceInventory(raw: unknown): Inventory {
  const inv = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  const devices = inv.devices;
  return {
    devices: Array.isArray(devices) ? devices.map(coerceDevice) : [],
    folders: Array.isArray(inv.folders) ? (inv.folders as string[]) : [],
  };
}

export function emptyDevice(kind: DeviceKind = "ssh"): Device {
  return {
    id: newId(),
    name: "",
    kind,
    vendor: kind === "local" ? "linux" : "generic",
    host: kind === "ssh" || kind === "api" ? "" : null,
    port: kind === "ssh" ? 22 : null,
    serial_path: kind === "serial" ? "/dev/ttyUSB0" : null,
    baud: kind === "serial" ? 9600 : null,
    api_base_url: kind === "api" ? "https://" : null,
    api_controller: kind === "api" ? "generic" : null,
    auth_profile_id: null,
    folder: null,
    tags: [],
    accent: "#2ee6a6",
    syntax_highlight: true,
    quick_copy: true,
    legacy_ssh: false,
    jump_host: null,
    shell: kind === "local" ? "/bin/bash" : null,
    notes: null,
  };
}
