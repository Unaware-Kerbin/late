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

export type ChatBackend = "local" | "cursor" | "ollama" | "llamacpp" | "anthropic" | "gemini" | "azure" | "mcp";

export type InferenceEngineBackend = "local" | "llamacpp" | "ollama";

export const LOCAL_INFERENCE_URL: Record<InferenceEngineBackend, string> = {
  local: "http://127.0.0.1:8000/v1",
  llamacpp: "http://127.0.0.1:8080/v1",
  ollama: "http://127.0.0.1:11434/v1",
};

export interface AppSettings {
  bind: string;
  vllm_base_url: string;
  vllm_model: string;
  /** Extra vLLM URL on your network. Chat uses `vllm_base_url` (this computer or this field). */
  vllm_remote_url?: string;
  cursor_model: string;
  ollama_base_url: string;
  ollama_model: string;
  ollama_remote_url?: string;
  llama_cpp_base_url: string;
  llama_cpp_model: string;
  llama_cpp_remote_url?: string;
  anthropic_base_url: string;
  anthropic_model: string;
  gemini_base_url: string;
  gemini_model: string;
  azure_base_url: string;
  azure_deployment: string;
  azure_api_version: string;
  default_backend: ChatBackend | string;
  scrollback_lines: number;
  turn_timeout_secs: number;
  max_agent_rounds: number;
  pcap_dir?: string;
  log_dir?: string;
  api_insecure_tls?: boolean;
  cloud_chat_enabled?: boolean;
  /** Extra hostnames for air-gapped OpenAI-compatible servers (not RFC1918 / .internal). */
  private_inference_hosts?: string;
  /** Optional MCP program. Folder on this computer and/or HTTP address. Off by default. */
  mcp_enabled?: boolean;
  mcp_cwd?: string;
  mcp_command?: string;
  mcp_args?: string;
  /** Streamable HTTP MCP URL. Empty = spawn the folder on this computer. */
  mcp_url?: string;
  /** When more than one GPU is on this computer, Local Start uses all of them. Default true. */
  use_all_gpus?: boolean;
}

export function coerceChatBackend(v: unknown): ChatBackend {
  const s = String(v ?? "").toLowerCase().replace(/[\s_]+/g, "-");
  if (s === "cursor" || s === "ollama" || s === "llamacpp" || s === "anthropic" || s === "gemini" || s === "azure" || s === "mcp") {
    return s;
  }
  if (s === "llama.cpp" || s === "llama-cpp") return "llamacpp";
  if (s === "claude") return "anthropic";
  if (s === "google" || s === "vertex") return "gemini";
  if (s === "azure-openai" || s === "azureopenai") return "azure";
  return "local";
}

export function backendLabel(backend: ChatBackend): string {
  if (backend === "ollama") return "Ollama";
  if (backend === "llamacpp") return "llama.cpp";
  if (backend === "cursor") return "Cursor SDK";
  if (backend === "anthropic") return "Anthropic Claude";
  if (backend === "gemini") return "Gemini";
  if (backend === "azure") return "Azure";
  if (backend === "mcp") return "MCP";
  return "vLLM";
}

export function isInferenceEngine(backend: ChatBackend): backend is InferenceEngineBackend {
  return backend === "local" || backend === "llamacpp" || backend === "ollama";
}

export function isLoopbackInferenceUrl(raw: string): boolean {
  try {
    const host = new URL(raw.trim()).hostname.replace(/^\[|\]$/g, "").toLowerCase();
    return (
      host === "localhost" ||
      host === "localhost.localdomain" ||
      host === "::1" ||
      host === "0.0.0.0" ||
      /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(host)
    );
  } catch {
    return false;
  }
}

export function inferenceHostLabel(raw: string): string {
  try {
    const u = new URL(raw.trim());
    const host = u.hostname || raw.trim();
    if (u.port && u.port !== "80" && u.port !== "443") return `${host}:${u.port}`;
    return host;
  } catch {
    return raw.trim() || "server";
  }
}

function activeUrl(settings: AppSettings, backend: InferenceEngineBackend): string {
  if (backend === "llamacpp") return settings.llama_cpp_base_url;
  if (backend === "ollama") return settings.ollama_base_url;
  return settings.vllm_base_url;
}

function storedRemoteUrl(settings: AppSettings, backend: InferenceEngineBackend): string {
  if (backend === "llamacpp") return settings.llama_cpp_remote_url?.trim() ?? "";
  if (backend === "ollama") return settings.ollama_remote_url?.trim() ?? "";
  return settings.vllm_remote_url?.trim() ?? "";
}

/** Saved LAN URL, or the active URL when it is already not this computer. */
export function remoteInferenceUrl(settings: AppSettings | null, backend: InferenceEngineBackend): string {
  if (!settings) return "";
  const stored = storedRemoteUrl(settings, backend);
  if (stored) return stored;
  const active = activeUrl(settings, backend).trim();
  if (active && !isLoopbackInferenceUrl(active)) return active;
  return "";
}

export function usingRemoteInference(settings: AppSettings | null, backend: InferenceEngineBackend): boolean {
  if (!settings) return false;
  const active = activeUrl(settings, backend).trim();
  return Boolean(active) && !isLoopbackInferenceUrl(active);
}

export function withInferenceServer(
  settings: AppSettings,
  backend: InferenceEngineBackend,
  next: { base: string; remote: string },
): AppSettings {
  if (backend === "llamacpp") {
    return { ...settings, llama_cpp_base_url: next.base, llama_cpp_remote_url: next.remote };
  }
  if (backend === "ollama") {
    return { ...settings, ollama_base_url: next.base, ollama_remote_url: next.remote };
  }
  return { ...settings, vllm_base_url: next.base, vllm_remote_url: next.remote };
}

/** Example Streamable HTTP URL when MCP_PORT is left at the default. Late Settings is the source of truth. */
export const DEFAULT_MCP_HTTP_URL = "http://127.0.0.1:8790/mcp";

export type McpPickMode = "off" | "stdio" | "http";

/** Agent-pane MCP menu: Off, this computer (stdio folder), or HTTP address. */
export function mcpPickMode(settings: AppSettings | null): McpPickMode {
  if (!settings?.mcp_enabled) return "off";
  if (settings.mcp_url?.trim()) return "http";
  return "stdio";
}

export function mcpPickLabel(settings: AppSettings | null): string {
  const mode = mcpPickMode(settings);
  if (mode === "off") return "MCP off";
  if (mode === "http") {
    const url = settings?.mcp_url?.trim() ?? "";
    return url ? `MCP · ${inferenceHostLabel(url)}` : "MCP · HTTP";
  }
  return "MCP · this computer";
}

/** Second Agent control when the backend is MCP (same idea as Local / Add server). */
export function mcpLocationLabel(settings: AppSettings | null): string {
  const mode = mcpPickMode(settings);
  if (mode === "http") {
    const url = settings?.mcp_url?.trim() ?? "";
    return url ? inferenceHostLabel(url) : "HTTP";
  }
  return "This computer";
}

export function withMcpPick(settings: AppSettings, next: { enabled: boolean; url: string }): AppSettings {
  return { ...settings, mcp_enabled: next.enabled, mcp_url: next.url.trim() };
}

/** Agent status line: MCP HTTP initialize/tools/list on the Settings URL. */
export function mcpStatusLine(mcp?: {
  ok?: boolean;
  enabled?: boolean;
  tools?: string[];
  message?: string;
}): string {
  if (!mcp?.enabled) return "MCP off";
  if (mcp.ok) {
    const n = mcp.tools?.length ?? 0;
    return `MCP on · ${n} tool${n === 1 ? "" : "s"}`;
  }
  const msg = mcp.message?.trim() ?? "";
  if (/GUI logos unavailable|logo fetch|fetchLogo/i.test(msg)) {
    return "GUI logos unavailable";
  }
  return "MCP error";
}

export function coerceSettings(raw: unknown): AppSettings | null {
  if (!raw || typeof raw !== "object") return null;
  const r = raw as Record<string, unknown>;
  const str = (...keys: string[]): string | undefined => {
    for (const k of keys) {
      const v = r[k];
      if (typeof v === "string" && v) return v;
    }
    return undefined;
  };
  const num = (...keys: string[]): number | undefined => {
    for (const k of keys) {
      const v = r[k];
      if (typeof v === "number" && Number.isFinite(v)) return v;
    }
    return undefined;
  };
  const flag = (...keys: string[]): boolean | undefined => {
    for (const k of keys) {
      const v = r[k];
      if (typeof v === "boolean") return v;
    }
    return undefined;
  };
  return {
    bind: str("bind") ?? "127.0.0.1:7420",
    vllm_base_url: str("vllm_base_url", "vllmBaseUrl") ?? "http://127.0.0.1:8000/v1",
    vllm_model: str("vllm_model", "vllmModel") ?? "local",
    vllm_remote_url: str("vllm_remote_url", "vllmRemoteUrl") ?? "",
    cursor_model: str("cursor_model", "cursorModel") ?? "composer-2.5",
    ollama_base_url: str("ollama_base_url", "ollamaBaseUrl") ?? "http://127.0.0.1:11434/v1",
    ollama_model: str("ollama_model", "ollamaModel") ?? "",
    ollama_remote_url: str("ollama_remote_url", "ollamaRemoteUrl") ?? "",
    llama_cpp_base_url: str("llama_cpp_base_url", "llamaCppBaseUrl") ?? "http://127.0.0.1:8080/v1",
    llama_cpp_model: str("llama_cpp_model", "llamaCppModel") ?? "",
    llama_cpp_remote_url: str("llama_cpp_remote_url", "llamaCppRemoteUrl") ?? "",
    anthropic_base_url: str("anthropic_base_url", "anthropicBaseUrl") ?? "https://api.anthropic.com",
    anthropic_model: str("anthropic_model", "anthropicModel") ?? "claude-sonnet-4-5",
    gemini_base_url: str("gemini_base_url", "geminiBaseUrl") ?? "https://generativelanguage.googleapis.com",
    gemini_model: str("gemini_model", "geminiModel") ?? "gemini-2.5-flash",
    azure_base_url: str("azure_base_url", "azureBaseUrl") ?? "",
    azure_deployment: str("azure_deployment", "azureDeployment") ?? "",
    azure_api_version: str("azure_api_version", "azureApiVersion") ?? "2024-10-21",
    default_backend: coerceChatBackend(str("default_backend", "defaultBackend")),
    scrollback_lines: num("scrollback_lines", "scrollbackLines") ?? 32_000,
    turn_timeout_secs: num("turn_timeout_secs", "turnTimeoutSecs") ?? 90,
    max_agent_rounds: num("max_agent_rounds", "maxAgentRounds") ?? 50,
    pcap_dir: str("pcap_dir", "pcapDir"),
    log_dir: str("log_dir", "logDir"),
    api_insecure_tls: flag("api_insecure_tls", "apiInsecureTls") ?? false,
    cloud_chat_enabled: flag("cloud_chat_enabled", "cloudChatEnabled") ?? false,
    private_inference_hosts: str("private_inference_hosts", "privateInferenceHosts") ?? "",
    mcp_enabled: flag("mcp_enabled", "mcpEnabled") ?? false,
    mcp_cwd: str("mcp_cwd", "mcpCwd") ?? "",
    mcp_command: str("mcp_command", "mcpCommand") ?? "",
    mcp_args: str("mcp_args", "mcpArgs") ?? "",
    mcp_url: str("mcp_url", "mcpUrl", "mcp_base_url", "mcpBaseUrl") ?? "",
    use_all_gpus: flag("use_all_gpus", "useAllGpus") ?? true,
  };
}

/** Flex axis stored on the tree: row (`right`) or column (`down`). */
export type SplitDir = "right" | "down";
/** Where a new pane is placed relative to the focused one. */
export type SplitPlacement = "left" | "right" | "up" | "down";

export type SplitNode =
  | { type: "leaf"; paneId: string }
  | { type: "split"; dir: SplitDir; ratio: number; a: SplitNode; b: SplitNode };

export interface PaneState {
  id: string;
  kind: SessionKind | "empty" | "stage";
  stageId?: string;
  stageFormat?: string;
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
  speaker?: string;
  label?: string;
  nickname?: string;
  logoDataUrl?: string;
  skipped?: boolean;
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
  if (s === "host" || s === "ip" || s === "tcp") return "ssh";
  return "ssh";
}

/** Collapse empty / `.` / `..` segments. Empty input becomes ungrouped (`null`). */
export function normalizeFolderPath(raw: string | null | undefined): string | null {
  const path = (raw ?? "")
    .split("/")
    .map((p) => p.trim())
    .filter((p) => p && p !== "." && p !== "..")
    .join("/");
  return path || null;
}

/** True when `path` is `ancestor` or a descendant (`ancestor/...`). "NY" does not match "NYC". */
export function folderPathIsUnder(path: string, ancestor: string): boolean {
  if (!ancestor) return path === "";
  return path === ancestor || path.startsWith(`${ancestor}/`);
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
    folder: normalizeFolderPath(pickStr(d, "folder")),
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

function coerceFolderList(raw: unknown): string[] {
  if (!Array.isArray(raw)) return [];
  const out: string[] = [];
  for (const item of raw) {
    const path = normalizeFolderPath(typeof item === "string" ? item : String(item ?? ""));
    if (!path) continue;
    let acc = "";
    for (const part of path.split("/")) {
      acc = acc ? `${acc}/${part}` : part;
      if (!out.includes(acc)) out.push(acc);
    }
  }
  return out;
}

export function coerceInventory(raw: unknown): Inventory {
  const inv = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  const devices = Array.isArray(inv.devices) ? inv.devices.map(coerceDevice) : [];
  // RPC snake_to_camel leaves `folders` unchanged (no `_`). Accept aliases if a caller skips remapping.
  const folders = coerceFolderList(pickRaw(inv, "folders", "folder_list", "folderList"));
  for (const d of devices) {
    if (!d.folder) continue;
    let acc = "";
    for (const part of d.folder.split("/")) {
      acc = acc ? `${acc}/${part}` : part;
      if (!folders.includes(acc)) folders.push(acc);
    }
  }
  return { devices, folders };
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
