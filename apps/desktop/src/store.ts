import { useRef, useSyncExternalStore } from "react";
import {
  applyChrome,
  loadAppearance,
  loadTermFont,
  termFontCss,
  TERM_FONT_SIZE,
  type Appearance,
  type DensityId,
  type FontId,
  type LayoutId,
  type RadiusId,
  type TermFontId,
  type ThemeId,
} from "./appearance";
import {
  DEFAULT_TERM_HIGHLIGHTS,
  loadTermHighlights,
  persistTermHighlights,
  sanitizeHighlights,
  type TermHighlightSettings,
} from "./termHighlight";
import {
  CHAT_H,
  CHAT_W,
  MODELS_H,
  SIDE_W,
  clampChatHeight,
  clampModelsH,
  clampShellWidth,
  defaultOrder,
  isAgentStacked,
  layoutFromOrder,
  loadShellOrder,
  moveShellPane,
  type ShellPaneId,
} from "./shell";
import { isHostKeyError, rpc, textToB64 } from "./lib/rpc";
import {
  coerceAuth,
  coerceInventory,
  coerceSession,
  coerceSettings,
  emptyDevice,
  newId,
  normalizeFolderPath,
  type AppSettings,
  type AuthProfile,
  type CaptureRecord,
  type CommandCollection,
  type Device,
  type DiffLine,
  type HostKeyPrompt,
  type Inventory,
  type PaneState,
  type ApprovalPrompt,
  type SessionInfo,
  type SessionKind,
  type SplitNode,
  type TabState,
} from "./types";

export type Toast = { id: string; kind: "error" | "info" | "ok"; text: string };

export type AppState = {
  daemonOk: boolean;
  daemonError: string | null;
  sidecarOk: boolean;
  inventory: Inventory;
  auth: AuthProfile[];
  settings: AppSettings | null;
  sessions: SessionInfo[];
  tabs: TabState[];
  panes: Record<string, PaneState>;
  activeTabId: string | null;
  focusedPaneId: string | null;
  selectedDeviceId: string | null;
  hostKey: HostKeyPrompt | null;
  approval: ApprovalPrompt | null;
  paletteOpen: boolean;
  importOpen: boolean;
  settingsOpen: boolean;
  keysOpen: boolean;
  collectionsOpen: boolean;
  captureOpen: boolean;
  authOpen: boolean;
  deviceEditor: Device | null;
  chatOpen: boolean;
  agentSeed: { text: string; autoSend?: boolean } | null;
  toasts: Toast[];
  collections: CommandCollection[];
  captures: CaptureRecord[];
  lastDiff: DiffLine[] | null;
  uiScale: number;
  termFontSize: number;
  termFont: TermFontId;
  termFontCustom: string;
  theme: ThemeId;
  density: DensityId;
  radius: RadiusId;
  uiFont: FontId;
  layout: LayoutId;
  termHighlights: TermHighlightSettings;
  shellOrder: ShellPaneId[];
  sideW: number;
  chatW: number;
  chatH: number;
  chatModelsH: number;
  providerStatus: Record<string, boolean>;
};

function freshPane(): PaneState {
  return { id: newId(), kind: "empty", disconnected: false, searchOpen: false };
}

function bootstrap(): AppState {
  const pane = freshPane();
  const tab: TabState = { id: newId(), title: "Workspace", tree: { type: "leaf", paneId: pane.id } };
  return {
    daemonOk: false,
    daemonError: "connecting…",
    sidecarOk: false,
    inventory: { devices: [], folders: [] },
    auth: [],
    settings: null,
    sessions: [],
    tabs: [tab],
    panes: { [pane.id]: pane },
    activeTabId: tab.id,
    focusedPaneId: pane.id,
    selectedDeviceId: null,
    hostKey: null,
    approval: null,
    paletteOpen: false,
    importOpen: false,
    settingsOpen: false,
    keysOpen: false,
    collectionsOpen: false,
    captureOpen: false,
    authOpen: false,
    deviceEditor: null,
    chatOpen: true,
    agentSeed: null,
    toasts: [],
    collections: [],
    captures: [],
    lastDiff: null,
    uiScale: readStored("late.uiScale", 1.15, 0.85, 2),
    termFontSize: readStored("late.termFontSize", TERM_FONT_SIZE.fallback, TERM_FONT_SIZE.min, TERM_FONT_SIZE.max),
    ...(() => {
      const a = loadAppearance();
      const tf = loadTermFont();
      return {
        theme: a.theme,
        density: a.density,
        radius: a.radius,
        uiFont: a.font,
        layout: a.layout,
        termFont: tf.termFont,
        termFontCustom: tf.termFontCustom,
        shellOrder: loadShellOrder(a.layout),
        sideW: readStored("late.sideW", SIDE_W.fallback, SIDE_W.min, SIDE_W.max),
        chatW: readStored("late.chatW", CHAT_W.fallback, CHAT_W.min, CHAT_W.max),
        chatH: readStored("late.chatH", CHAT_H.fallback, CHAT_H.min, CHAT_H.max),
        chatModelsH: readModelsH(),
      };
    })(),
    termHighlights: loadTermHighlights(),
    providerStatus: {},
  };
}

function readStored(key: string, fallback: number, min: number, max: number): number {
  try {
    const n = Number(localStorage.getItem(key));
    if (!Number.isFinite(n)) return fallback;
    return Math.min(max, Math.max(min, n));
  } catch {
    return fallback;
  }
}

export function applyAppearance(s: AppState = state) {
  try {
    document.documentElement.style.fontSize = `${(13 * s.uiScale).toFixed(1)}px`;
    document.documentElement.style.setProperty("--term-font-size", `${s.termFontSize}px`);
    document.documentElement.style.setProperty("--term-font-family", termFontCss(s.termFont, s.termFontCustom));
    localStorage.setItem("late.uiScale", String(s.uiScale));
    localStorage.setItem("late.termFontSize", String(s.termFontSize));
    localStorage.setItem("late.termFont", s.termFont);
    localStorage.setItem("late.termFontCustom", s.termFontCustom);
    applyChrome({
      theme: s.theme,
      density: s.density,
      radius: s.radius,
      font: s.uiFont,
      layout: s.layout,
    });
    persistShell(s);
  } catch {
    /* ignore */
  }
}

function readModelsH(): number {
  try {
    const n = Number(localStorage.getItem("late.chatModelsH"));
    if (!Number.isFinite(n)) return MODELS_H.fallback;
    if (n <= 0) return 0;
    return Math.min(MODELS_H.max, Math.max(MODELS_H.min, n));
  } catch {
    return MODELS_H.fallback;
  }
}

function persistShell(s: AppState) {
  try {
    localStorage.setItem("late.sideW", String(s.sideW));
    localStorage.setItem("late.chatW", String(s.chatW));
    localStorage.setItem("late.chatH", String(s.chatH));
    localStorage.setItem("late.chatModelsH", String(s.chatModelsH));
    localStorage.setItem("late.shellOrder", s.shellOrder.join(","));
  } catch {
    /* ignore */
  }
}

export function bumpUiScale(delta: number) {
  setState((s) => {
    const uiScale = Math.min(2, Math.max(0.85, Number((s.uiScale + delta).toFixed(2))));
    const next = { ...s, uiScale };
    queueMicrotask(() => applyAppearance(next));
    return next;
  });
}

export function bumpTermFont(delta: number) {
  setState((s) => {
    const termFontSize = Math.min(TERM_FONT_SIZE.max, Math.max(TERM_FONT_SIZE.min, s.termFontSize + delta));
    const next = { ...s, termFontSize };
    queueMicrotask(() => applyAppearance(next));
    return next;
  });
}

export function resetAppearance() {
  setState((s) => {
    const a = {
      theme: "midnight" as const,
      density: "cozy" as const,
      radius: "soft" as const,
      uiFont: "plex" as const,
      layout: "chat-right" as const,
    };
    const next = {
      ...s,
      uiScale: 1.15,
      termFontSize: TERM_FONT_SIZE.fallback,
      termFont: "system" as const,
      termFontCustom: "",
      ...a,
      shellOrder: defaultOrder(a.layout),
      sideW: SIDE_W.fallback,
      chatW: CHAT_W.fallback,
      chatH: CHAT_H.fallback,
      chatModelsH: MODELS_H.fallback,
    };
    queueMicrotask(() => applyAppearance(next));
    return next;
  });
}

export function setAppearance(uiScale?: number, termFontSize?: number) {
  setState((s) => {
    const next = {
      ...s,
      uiScale: uiScale ?? s.uiScale,
      termFontSize: termFontSize ?? s.termFontSize,
    };
    queueMicrotask(() => applyAppearance(next));
    return next;
  });
}

export function setTermType(patch: { termFont?: TermFontId; termFontCustom?: string; termFontSize?: number }) {
  setState((s) => {
    const next = {
      ...s,
      termFont: patch.termFont ?? s.termFont,
      termFontCustom: patch.termFontCustom !== undefined ? patch.termFontCustom : s.termFontCustom,
      termFontSize: patch.termFontSize ?? s.termFontSize,
    };
    queueMicrotask(() => applyAppearance(next));
    return next;
  });
}

export function patchAppearance(patch: Partial<Appearance> & { uiFont?: FontId }) {
  setState((s) => {
    const next = {
      ...s,
      theme: patch.theme ?? s.theme,
      density: patch.density ?? s.density,
      radius: patch.radius ?? s.radius,
      uiFont: patch.uiFont ?? patch.font ?? s.uiFont,
      layout: patch.layout ?? s.layout,
      shellOrder: patch.layout ? defaultOrder(patch.layout) : s.shellOrder,
    };
    queueMicrotask(() => applyAppearance(next));
    return next;
  });
}

export function setTermHighlights(next: TermHighlightSettings | null) {
  setState((s) => {
    const termHighlights = sanitizeHighlights(next ?? structuredClone(DEFAULT_TERM_HIGHLIGHTS));
    persistTermHighlights(termHighlights);
    return { ...s, termHighlights };
  });
}

export function setShellWidth(id: "side" | "chat", px: number) {
  setState((s) => {
    const next = id === "chat" ? { ...s, chatW: clampShellWidth("chat", px) } : { ...s, sideW: clampShellWidth("side", px) };
    persistShell(next);
    return next;
  });
}

export function setChatHeight(px: number) {
  setState((s) => {
    const next = { ...s, chatH: clampChatHeight(px) };
    persistShell(next);
    return next;
  });
}

export function setChatModelsH(px: number) {
  setState((s) => {
    const chatModelsH = px <= 40 ? 0 : clampModelsH(px);
    const next = { ...s, chatModelsH };
    persistShell(next);
    return next;
  });
}

export function widenChat() {
  const px = typeof window !== "undefined" ? Math.round(window.innerWidth * 0.52) : 720;
  setShellWidth("chat", px);
}

export function dockAgent(where: "top" | "row") {
  setState((s) => {
    const layout: LayoutId = where === "top" ? "agent-top" : "chat-right";
    const next = {
      ...s,
      layout,
      shellOrder: defaultOrder(layout),
      chatH: where === "top" ? Math.max(s.chatH, 380) : s.chatH,
    };
    persistShell(next);
    queueMicrotask(() => applyAppearance(next));
    return next;
  });
}

export function dropShellPane(from: ShellPaneId, to: ShellPaneId) {
  setState((s) => {
    if (isAgentStacked(s.layout)) {
      if (from === "chat") {
        const row = s.shellOrder.filter((id) => id !== "chat");
        const shellOrder = moveShellPane(["chat", ...row], "chat", to);
        const next = { ...s, shellOrder, layout: layoutFromOrder(shellOrder) };
        persistShell(next);
        queueMicrotask(() => applyAppearance(next));
        return next;
      }
      const row = s.shellOrder.filter((id) => id !== "chat");
      const moved = moveShellPane(row, from, to);
      const next = { ...s, shellOrder: (["chat", ...moved] as ShellPaneId[]) };
      persistShell(next);
      return next;
    }
    const shellOrder = moveShellPane(s.shellOrder, from, to);
    const layout = layoutFromOrder(shellOrder);
    const next = { ...s, shellOrder, layout };
    persistShell(next);
    queueMicrotask(() => applyAppearance(next));
    return next;
  });
}

let state: AppState = bootstrap();
const listeners = new Set<() => void>();

function emit() {
  listeners.forEach((l) => l());
}

export function getState(): AppState {
  return state;
}

export function setState(patch: Partial<AppState> | ((s: AppState) => Partial<AppState>)) {
  const next = typeof patch === "function" ? patch(state) : patch;
  state = { ...state, ...next };
  emit();
}

function snapshotsEqual(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true;
  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (!Object.is(a[i], b[i])) return false;
    return true;
  }
  return false;
}

export function useApp<T>(sel: (s: AppState) => T): T {
  const cached = useRef<T>();
  const ready = useRef(false);
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => {
      const next = sel(state);
      if (ready.current && snapshotsEqual(cached.current, next)) return cached.current as T;
      cached.current = next;
      ready.current = true;
      return next;
    },
  );
}

export function toast(kind: Toast["kind"], text: string) {
  const id = newId();
  setState((s) => ({ ...s, toasts: [...s.toasts, { id, kind, text }].slice(-6) }));
  window.setTimeout(() => {
    setState((s) => ({ ...s, toasts: s.toasts.filter((t) => t.id !== id) }));
  }, 6000);
}

function errText(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

function focusedPane(): PaneState | undefined {
  return state.focusedPaneId ? state.panes[state.focusedPaneId] : undefined;
}

function activeTab(): TabState | undefined {
  return state.tabs.find((t) => t.id === state.activeTabId);
}

function unwrapList<T>(raw: unknown): T[] {
  if (Array.isArray(raw)) return raw as T[];
  if (raw && typeof raw === "object") {
    const o = raw as Record<string, unknown>;
    for (const k of ["collections", "captures", "items", "sessions"]) {
      if (Array.isArray(o[k])) return o[k] as T[];
    }
  }
  return [];
}

export async function refreshAll() {
  try {
    await rpc.connect();
    const [inv, auth, sessions, settings, collections, captures, providerStatus] = await Promise.all([
      rpc.call<Inventory>("inventory.list").catch(() => ({ devices: [] as Device[], folders: [] as string[] })),
      rpc.call<AuthProfile[]>("auth.list").catch(() => [] as AuthProfile[]),
      rpc.call<SessionInfo[]>("session.list").catch(() => [] as SessionInfo[]),
      rpc.call<AppSettings>("settings.get").catch(() => null),
      rpc.call<unknown>("collections.list").catch(() => []),
      rpc.call<unknown>("capture.list").catch(() => []),
      rpc.call<Record<string, boolean>>("providers.status").catch(() => ({})),
    ]);
    setState({
      daemonOk: true,
      daemonError: null,
      inventory: coerceInventory(inv),
      auth: Array.isArray(auth) ? auth.map(coerceAuth) : [],
      sessions: Array.isArray(sessions) ? sessions.map(coerceSession) : [],
      settings: coerceSettings(settings),
      collections: unwrapList<CommandCollection>(collections),
      captures: unwrapList<CaptureRecord>(captures),
      providerStatus: providerStatus ?? {},
    });
  } catch (err) {
    setState({ daemonOk: false, daemonError: errText(err) });
  }
}

function persistDevice(device: Device): Device {
  return { ...device, folder: normalizeFolderPath(device.folder) };
}

export async function upsertDevice(device: Device) {
  try {
    const next = persistDevice(device);
    await rpc.call("inventory.upsert", { device: next, ...next });
    toast("ok", `saved ${next.name}`);
    setState({ deviceEditor: null });
    await refreshAll();
  } catch (err) {
    toast("error", errText(err));
  }
}

export async function saveDeviceWithLogin(
  device: Device,
  login?: { username: string; password?: string; keyPath?: string | null },
) {
  try {
    let next = persistDevice(device);
    if (device.kind === "ssh") {
      const user = login?.username?.trim() ?? "";
      if (!user) {
        toast("error", "Username is required");
        return;
      }
      const hasPw = Boolean(login?.password);
      const hasKey = Boolean(login?.keyPath?.trim());
      const existing = getState().auth.find((p) => p.id === device.auth_profile_id);
      if (!hasPw && !hasKey && !existing?.has_password && !existing?.key_path) {
        toast("error", "Enter a password (or key path) so Late can log in without a prompt");
        return;
      }
      const id = device.auth_profile_id || newId();
      const profile: AuthProfile = {
        id,
        name: `${device.name || device.host || user} login`,
        username: user,
        key_path: login?.keyPath?.trim() || existing?.key_path || null,
        use_agent: !hasPw && !hasKey && !existing?.has_password,
        has_password: hasPw || Boolean(existing?.has_password),
      };
      await rpc.call("auth.upsert", {
        profile,
        ...profile,
        password: login?.password || undefined,
      });
      next = persistDevice({ ...next, auth_profile_id: id });
    }
    await rpc.call("inventory.upsert", { device: next, ...next });
    toast("ok", `saved ${next.name}`);
    setState({ deviceEditor: null });
    await refreshAll();
  } catch (err) {
    toast("error", errText(err));
  }
}

export async function deleteDevice(id: string) {
  try {
    await rpc.call("inventory.delete", { id });
    await refreshAll();
  } catch (err) {
    toast("error", errText(err));
  }
}

export async function upsertFolder(path: string): Promise<boolean> {
  const next = normalizeFolderPath(path);
  if (!next) {
    toast("error", "Folder name is empty");
    return false;
  }
  try {
    await rpc.call("inventory.folder.upsert", { path: next, folder: next });
    await refreshAll();
    return true;
  } catch (err) {
    toast("error", errText(err));
    return false;
  }
}

export async function renameFolder(from: string, to: string): Promise<boolean> {
  const src = normalizeFolderPath(from);
  const dest = normalizeFolderPath(to);
  if (!src || !dest) {
    toast("error", "Folder name is empty");
    return false;
  }
  try {
    await rpc.call("inventory.folder.rename", { from: src, to: dest, old: src });
    await refreshAll();
    return true;
  } catch (err) {
    toast("error", errText(err));
    return false;
  }
}

export async function deleteFolder(path: string): Promise<boolean> {
  const next = normalizeFolderPath(path);
  if (!next) {
    toast("error", "Folder name is empty");
    return false;
  }
  try {
    await rpc.call("inventory.folder.delete", { path: next, folder: next });
    await refreshAll();
    return true;
  } catch (err) {
    toast("error", errText(err));
    return false;
  }
}

export async function moveDeviceToFolder(deviceId: string, folder: string | null) {
  const device = getState().inventory.devices.find((d) => d.id === deviceId);
  if (!device) return;
  const next = persistDevice({ ...device, folder: normalizeFolderPath(folder) });
  try {
    await rpc.call("inventory.upsert", { device: next, ...next });
    await refreshAll();
  } catch (err) {
    toast("error", errText(err));
  }
}

export async function upsertAuth(profile: AuthProfile, password?: string) {
  try {
    await rpc.call("auth.upsert", { profile, ...profile, password: password || undefined });
    toast("ok", `auth profile ${profile.name} saved`);
    await refreshAll();
  } catch (err) {
    toast("error", errText(err));
  }
}

export async function deleteAuth(id: string) {
  try {
    await rpc.call("auth.delete", { id });
    await refreshAll();
  } catch (err) {
    toast("error", errText(err));
  }
}

function kindForDevice(d: Device, override?: SessionKind): SessionKind {
  if (override) return override;
  if (d.kind === "api") return "api";
  if (d.kind === "serial") return "serial";
  if (d.kind === "local") return "local";
  return "ssh";
}

function attachSession(info: SessionInfo, deviceId: string | undefined, kind: SessionKind) {
  const pane = focusedPane();
  const paneId = pane && (pane.kind === "empty" || pane.disconnected) ? pane.id : splitNewPane(info.name);
  setState((s) => ({
    ...s,
    hostKey: null,
    panes: {
      ...s.panes,
      [paneId]: {
        ...s.panes[paneId],
        kind,
        session: info,
        deviceId,
        disconnected: false,
        disconnectReason: undefined,
      },
    },
    tabs: s.tabs.map((t) => (t.id === s.activeTabId ? { ...t, title: info.name } : t)),
    focusedPaneId: paneId,
  }));
}

export async function openSession(
  device: Device,
  kind?: SessionKind,
  hostKeyFlags?: { acceptUnknownHost?: boolean; replaceHostKey?: boolean },
) {
  const sessionKind = kindForDevice(device, kind);
  try {
    const info = await rpc.call<SessionInfo>("session.open", {
      deviceId: device.id,
      device_id: device.id,
      kind: sessionKind,
      cols: 120,
      rows: 36,
      acceptUnknownHost: hostKeyFlags?.acceptUnknownHost,
      replaceHostKey: hostKeyFlags?.replaceHostKey,
      accept_unknown_host: hostKeyFlags?.acceptUnknownHost,
      replace_host_key: hostKeyFlags?.replaceHostKey,
    });
    attachSession(info, device.id, sessionKind);
    await refreshAll();
  } catch (err) {
    const hk = isHostKeyError(err);
    if (hk) {
      setState({
        hostKey: {
          host: hk.host || device.host || device.name,
          presented: hk.presented ?? "unknown fingerprint",
          pinned: hk.pinned,
          mismatch: hk.mismatch,
          retry: async () => {
            setState({ hostKey: null });
            const fp = hk.presented;
            if (hk.host && fp && fp !== "unknown fingerprint") {
              try {
                await rpc.call("known_hosts.accept", {
                  host: hk.host,
                  fingerprint: fp,
                  presented: fp,
                });
              } catch {
                /* session.open flags still pin if this method is missing */
              }
            }
            await openSession(device, sessionKind, {
              acceptUnknownHost: true,
              replaceHostKey: hk.mismatch,
            });
          },
        },
      });
      return;
    }
    toast("error", errText(err));
  }
}

export async function openLocal(shell?: string) {
  try {
    const info = await rpc.call<SessionInfo>("session.open", { kind: "local", shell, cols: 120, rows: 36 });
    attachSession(info, undefined, "local");
  } catch (err) {
    toast("error", errText(err));
  }
}

const userClosedSessions = new Set<string>();
const reconnectingSessions = new Set<string>();

function noteUserClosed(sessionId: string | undefined) {
  if (sessionId) userClosedSessions.add(sessionId);
}

export async function reconnectPane(paneId: string) {
  const pane = state.panes[paneId];
  if (!pane?.session) return;
  const oldId = pane.session.id;
  reconnectingSessions.add(oldId);
  try {
    const info = await rpc.call<SessionInfo>("session.reconnect", {
      sessionId: oldId,
      session_id: oldId,
    });
    setState((s) => ({
      ...s,
      panes: {
        ...s.panes,
        [paneId]: { ...s.panes[paneId], session: info, disconnected: false, disconnectReason: undefined },
      },
    }));
  } catch (err) {
    const hk = isHostKeyError(err);
    const device = state.inventory.devices.find((d) => d.id === pane.deviceId);
    if (hk && device) {
      setState({
        hostKey: {
          host: hk.host,
          presented: hk.presented ?? "",
          pinned: hk.pinned,
          mismatch: hk.mismatch,
          retry: async () => {
            setState({ hostKey: null });
            const fp = hk.presented;
            if (hk.host && fp && fp !== "unknown fingerprint") {
              try {
                await rpc.call("known_hosts.accept", {
                  host: hk.host,
                  fingerprint: fp,
                  presented: fp,
                });
              } catch {
                /* session.open flags still pin if this method is missing */
              }
            }
            await openSession(device, pane.session!.kind, {
              acceptUnknownHost: true,
              replaceHostKey: hk.mismatch,
            });
          },
        },
      });
      return;
    }
    toast("error", errText(err));
    markDisconnected(oldId, errText(err));
  } finally {
    reconnectingSessions.delete(oldId);
  }
}

export async function closePaneSession(paneId: string) {
  await closePane(paneId);
}

export async function sendInput(sessionId: string, text: string) {
  await rpc.call("session.input", { sessionId, session_id: sessionId, data: textToB64(text) });
}

export async function resizeSession(sessionId: string, cols: number, rows: number) {
  await rpc.call("session.resize", { sessionId, session_id: sessionId, cols, rows }).catch(() => undefined);
}

export async function sendBreak(sessionId: string) {
  try {
    await rpc.call("session.break", { id: sessionId, sessionId, session_id: sessionId });
  } catch (err) {
    toast("error", errText(err));
  }
}

export async function exportSession(sessionId: string, passphrase?: string) {
  try {
    const r = await rpc.call<{ path?: string }>("session.export", {
      sessionId,
      session_id: sessionId,
      encryptPassphrase: passphrase,
    });
    toast("ok", `exported ${r?.path ?? "session"}`);
  } catch (err) {
    toast("error", errText(err));
  }
}

export async function setLogging(sessionId: string, enabled: boolean) {
  try {
    await rpc.call("session.setLogging", { sessionId, session_id: sessionId, enabled });
    setState((s) => ({
      panes: Object.fromEntries(
        Object.entries(s.panes).map(([id, p]) => [
          id,
          p.session?.id === sessionId ? { ...p, logging: enabled } : p,
        ]),
      ),
    }));
    toast("info", enabled ? "session logging on" : "session logging off");
  } catch (err) {
    toast("error", errText(err));
  }
}

export async function saveCapture(name: string, sessionId: string) {
  try {
    const sb = await rpc.call<{ text?: string }>("session.scrollback", { sessionId, session_id: sessionId });
    await rpc.call("capture.save", { name, sessionId, session_id: sessionId, output: sb?.text ?? "" });
    toast("ok", "capture saved");
    await refreshAll();
  } catch (err) {
    toast("error", errText(err));
  }
}

export async function diffCaptures(a: string, b: string) {
  try {
    const lines = await rpc.call<DiffLine[]>("capture.diff", { a, b, idA: a, idB: b });
    setState({ lastDiff: lines, captureOpen: true });
  } catch (err) {
    toast("error", errText(err));
  }
}

export async function upsertCollection(col: CommandCollection) {
  try {
    await rpc.call("collections.upsert", { collection: col, ...col });
    await refreshAll();
  } catch (err) {
    toast("error", errText(err));
  }
}

export async function deleteCollection(id: string) {
  try {
    await rpc.call("collections.delete", { id });
    await refreshAll();
  } catch (err) {
    toast("error", errText(err));
  }
}

export async function runCollection(id: string) {
  const col = state.collections.find((c) => c.id === id);
  if (!col) {
    toast("error", "collection not found");
    return;
  }
  const pane = focusedPane();
  const sessionId = pane?.session?.id;
  if (!sessionId || pane?.kind === "pcap" || pane?.kind === "sftp" || pane?.kind === "api" || pane?.kind === "stage") {
    toast("error", "focus an SSH, serial, or local terminal first");
    return;
  }
  const cmds = (col.commands ?? []).map((c) => c.trim()).filter(Boolean);
  if (!cmds.length) {
    toast("error", "that collection has no commands");
    return;
  }
  try {
    for (const cmd of cmds) {
      await sendInput(sessionId, `${cmd}\r`);
      await new Promise((r) => window.setTimeout(r, 60));
    }
    toast("ok", `sent ${cmds.length} command${cmds.length === 1 ? "" : "s"} to ${pane.session?.name ?? "session"}`);
  } catch (err) {
    toast("error", errText(err));
  }
}

export async function saveProviderKey(name: string, key: string) {
  try {
    await rpc.call("providers.set", { name, key });
    toast("ok", `${name} key saved on your computer (0600 vault, not sent to the UI again)`);
    await refreshAll();
  } catch (err) {
    toast("error", errText(err));
  }
}

export async function clearProviderKey(name: string) {
  try {
    await rpc.call("providers.delete", { name });
    toast("ok", `${name} key cleared`);
    await refreshAll();
  } catch (err) {
    toast("error", errText(err));
  }
}

export type LocalModel = {
  id: string;
  complete: boolean;
  sizeBytes: number;
  note: string;
  recommended?: boolean;
  tp?: number;
};

export type GpuProfile = {
  vendor: string;
  discreteCount: number;
  vramGb: number;
  tpOk: boolean;
  summary: string;
};

export type InferenceStatus = {
  running: boolean;
  starting?: boolean;
  downloading?: boolean;
  models: string[];
  localModels?: LocalModel[];
  container?: string | null;
  downloadId?: string | null;
  serveModel?: string | null;
  gpu?: GpuProfile;
  detail: string;
  allowIntelCompose?: boolean;
  lateOwned?: boolean;
};

export async function inferenceStatus(engine?: string): Promise<InferenceStatus> {
  return rpc.call<InferenceStatus>("inference.status", engine ? { engine } : {});
}

export async function startInference(model?: string, engine?: string): Promise<InferenceStatus> {
  const id = (model ?? "").trim();
  if (!id) {
    throw new Error(
      engine === "llamacpp"
        ? "pick a GGUF above, then Start"
        : "pick Hugging Face weights in “Weights to serve”, then Start",
    );
  }
  toast(
    "info",
    engine === "llamacpp"
      ? `starting llama-server with ${id}`
      : `starting ${id} — first load can take several minutes`,
  );
  return rpc.call<InferenceStatus>("inference.start", {
    model: id,
    serveModel: id,
    engine: engine || "vllm",
  });
}

export async function stopInference(engine?: string): Promise<InferenceStatus> {
  toast("info", engine === "llamacpp" ? "stopping llama-server" : "stopping local inference");
  return rpc.call<InferenceStatus>("inference.stop", engine ? { engine } : {});
}

export async function downloadInference(model: string, engine?: string): Promise<InferenceStatus> {
  const kind = engine || "vllm";
  toast(
    "info",
    kind === "ollama"
      ? `pulling ${model} via local Ollama`
      : kind === "llamacpp"
        ? `downloading ${model} GGUF from Hugging Face`
        : `downloading ${model} into the Hugging Face cache`,
  );
  return rpc.call<InferenceStatus>("inference.download", { model, engine: kind });
}

export async function saveSettings(settings: AppSettings) {
  try {
    await rpc.call("settings.set", { settings, ...settings });
    toast("ok", "settings saved");
    await refreshAll();
  } catch (err) {
    toast("error", errText(err));
  }
}

export async function importFile(path: string, kind?: string) {
  try {
    const result = await rpc.call<{ devices?: Device[]; warnings?: string[] }>("import.file", { path, kind });
    const devices = result?.devices ?? [];
    for (const d of devices) {
      await rpc.call("inventory.upsert", { device: d, ...d });
    }
    toast("ok", `imported ${devices.length} devices`);
    if (result?.warnings?.length) toast("info", result.warnings.join("; "));
    setState({ importOpen: false });
    await refreshAll();
  } catch (err) {
    toast("error", errText(err));
  }
}

export function newTab(title = "New") {
  const pane = freshPane();
  const tab: TabState = { id: newId(), title, tree: { type: "leaf", paneId: pane.id } };
  setState((s) => ({
    ...s,
    tabs: [...s.tabs, tab],
    panes: { ...s.panes, [pane.id]: pane },
    activeTabId: tab.id,
    focusedPaneId: pane.id,
  }));
}

function paneIdsIn(node: SplitNode): string[] {
  if (node.type === "leaf") return [node.paneId];
  return [...paneIdsIn(node.a), ...paneIdsIn(node.b)];
}

function firstPaneId(node: SplitNode): string {
  return paneIdsIn(node)[0];
}

function treeHasPane(node: SplitNode, paneId: string | null): boolean {
  if (!paneId) return false;
  return paneIdsIn(node).includes(paneId);
}

function prunePane(node: SplitNode, paneId: string): SplitNode | null {
  if (node.type === "leaf") return node.paneId === paneId ? null : node;
  const a = prunePane(node.a, paneId);
  const b = prunePane(node.b, paneId);
  if (!a) return b;
  if (!b) return a;
  return { ...node, a, b };
}

function closeSessions(ids: string[]) {
  for (const pid of ids) {
    const pane = state.panes[pid];
    const sid = pane?.session?.id;
    if (sid) {
      noteUserClosed(sid);
      void rpc.call("session.close", { sessionId: sid, session_id: sid }).catch(() => undefined);
    }
  }
}

export function closeTab(id: string) {
  const tab = state.tabs.find((t) => t.id === id);
  const ids = tab ? paneIdsIn(tab.tree) : [];
  closeSessions(ids);
  setState((s) => {
    const rest = s.tabs.filter((t) => t.id !== id);
    const panes = { ...s.panes };
    for (const pid of ids) delete panes[pid];
    if (rest.length) {
      const activeTabId = s.activeTabId === id ? rest[rest.length - 1].id : s.activeTabId;
      const active = rest.find((t) => t.id === activeTabId) ?? rest[0];
      return {
        ...s,
        tabs: rest,
        panes,
        activeTabId: active.id,
        focusedPaneId: treeHasPane(active.tree, s.focusedPaneId)
          ? s.focusedPaneId
          : firstPaneId(active.tree),
      };
    }
    const pane = freshPane();
    const next: TabState = { id: newId(), title: "Workspace", tree: { type: "leaf", paneId: pane.id } };
    panes[pane.id] = pane;
    return { ...s, tabs: [next], panes, activeTabId: next.id, focusedPaneId: pane.id };
  });
}

export async function closePane(paneId: string) {
  const pane = state.panes[paneId];
  if (pane?.session) {
    noteUserClosed(pane.session.id);
    try {
      await rpc.call("session.close", {
        sessionId: pane.session.id,
        session_id: pane.session.id,
      });
    } catch (err) {
      toast("error", errText(err));
    }
  }
  setState((s) => {
    const panes = { ...s.panes };
    delete panes[paneId];
    const tabs: TabState[] = [];
    for (const t of s.tabs) {
      const tree = prunePane(t.tree, paneId);
      if (tree) tabs.push({ ...t, tree });
    }
    if (!tabs.length) {
      const empty = freshPane();
      const tab: TabState = { id: newId(), title: "Workspace", tree: { type: "leaf", paneId: empty.id } };
      panes[empty.id] = empty;
      return { ...s, tabs: [tab], panes, activeTabId: tab.id, focusedPaneId: empty.id };
    }
    const active = tabs.find((t) => t.id === s.activeTabId) ?? tabs[0];
    const focusedPaneId = treeHasPane(active.tree, s.focusedPaneId)
      ? s.focusedPaneId
      : firstPaneId(active.tree);
    return { ...s, tabs, panes, activeTabId: active.id, focusedPaneId };
  });
}

function mapTree(node: SplitNode, fn: (n: SplitNode) => SplitNode): SplitNode {
  const mapped = fn(node);
  if (mapped.type === "split") return { ...mapped, a: mapTree(mapped.a, fn), b: mapTree(mapped.b, fn) };
  return mapped;
}

export function splitKey(node: SplitNode): string {
  if (node.type === "leaf") return node.paneId;
  return `${node.dir}:${splitKey(node.a)}/${splitKey(node.b)}`;
}

export function setSplitRatio(tabId: string, key: string, ratio: number) {
  const r = Math.min(0.82, Math.max(0.18, ratio));
  setState((s) => ({
    tabs: s.tabs.map((t) => {
      if (t.id !== tabId) return t;
      return {
        ...t,
        tree: mapTree(t.tree, (n) =>
          n.type === "split" && splitKey(n) === key ? { ...n, ratio: r } : n,
        ),
      };
    }),
  }));
}

export function renameTab(id: string, title: string) {
  const next = title.trim();
  if (!next) return;
  setState((s) => ({
    tabs: s.tabs.map((t) => (t.id === id ? { ...t, title: next } : t)),
  }));
}

export function splitFocused(dir: "right" | "down") {
  const tab = activeTab();
  const focus = state.focusedPaneId;
  if (!tab || !focus) return;
  const pane = freshPane();
  const tree = mapTree(tab.tree, (n) => {
    if (n.type === "leaf" && n.paneId === focus) {
      return { type: "split", dir, ratio: 0.5, a: n, b: { type: "leaf", paneId: pane.id } };
    }
    return n;
  });
  setState((s) => ({
    ...s,
    panes: { ...s.panes, [pane.id]: pane },
    tabs: s.tabs.map((t) => (t.id === tab.id ? { ...t, tree } : t)),
    focusedPaneId: pane.id,
  }));
}

function splitNewPane(title: string): string {
  const pane = freshPane();
  const tab = activeTab();
  if (!tab) {
    newTab(title);
    return getState().focusedPaneId ?? pane.id;
  }
  setState((s) => ({
    ...s,
    panes: { ...s.panes, [pane.id]: pane },
    tabs: s.tabs.map((t) =>
      t.id === tab.id
        ? {
            ...t,
            title,
            tree: { type: "split", dir: "right", ratio: 0.5, a: t.tree, b: { type: "leaf", paneId: pane.id } },
          }
        : t,
    ),
    focusedPaneId: pane.id,
  }));
  return pane.id;
}

export function openPcapPane() {
  const focus = state.focusedPaneId;
  const cur = focus ? state.panes[focus] : undefined;
  // Never split or replace a live terminal — WebGL xterm goes black on that layout change.
  if (cur && cur.kind === "empty" && !cur.session) {
    setState((s) => ({
      panes: { ...s.panes, [cur.id]: { ...s.panes[cur.id], kind: "pcap" } },
      tabs: s.tabs.map((t) =>
        t.id === s.activeTabId ? { ...t, title: "Packet capture" } : t,
      ),
    }));
    return;
  }
  const pane = { ...freshPane(), kind: "pcap" as const };
  const tab: TabState = {
    id: newId(),
    title: "Packet capture",
    tree: { type: "leaf", paneId: pane.id },
  };
  setState((s) => ({
    tabs: [...s.tabs, tab],
    panes: { ...s.panes, [pane.id]: pane },
    activeTabId: tab.id,
    focusedPaneId: pane.id,
  }));
}

export function openStagePane(stageId?: string, format?: string) {
  const existing = Object.values(state.panes).find((p) => p.kind === "stage");
  if (existing) {
    const tab = state.tabs.find((t) => treeHasPane(t.tree, existing.id));
    setState((s) => ({
      panes: {
        ...s.panes,
        [existing.id]: {
          ...s.panes[existing.id],
          ...(stageId ? { stageId } : {}),
          ...(format ? { stageFormat: format } : {}),
        },
      },
      focusedPaneId: existing.id,
      activeTabId: tab?.id ?? s.activeTabId,
    }));
    return;
  }
  const focus = state.focusedPaneId;
  const cur = focus ? state.panes[focus] : undefined;
  if (cur && cur.kind === "empty" && !cur.session) {
    setState((s) => ({
      panes: {
        ...s.panes,
        [cur.id]: {
          ...s.panes[cur.id],
          kind: "stage",
          ...(stageId ? { stageId } : {}),
          ...(format ? { stageFormat: format } : {}),
        },
      },
      tabs: s.tabs.map((t) =>
        t.id === s.activeTabId ? { ...t, title: "Staging" } : t,
      ),
    }));
    return;
  }
  const pane = {
    ...freshPane(),
    kind: "stage" as const,
    ...(stageId ? { stageId } : {}),
    ...(format ? { stageFormat: format } : {}),
  };
  const tab: TabState = {
    id: newId(),
    title: "Staging",
    tree: { type: "leaf", paneId: pane.id },
  };
  setState((s) => ({
    tabs: [...s.tabs, tab],
    panes: { ...s.panes, [pane.id]: pane },
    activeTabId: tab.id,
    focusedPaneId: pane.id,
  }));
}

export function markDisconnected(sessionId: string, reason?: string) {
  setState((s) => {
    const panes = { ...s.panes };
    for (const [id, p] of Object.entries(panes)) {
      if (p.session?.id === sessionId) {
        panes[id] = {
          ...p,
          disconnected: true,
          disconnectReason: reason ?? "disconnected — press Enter to reconnect",
        };
      }
    }
    return { ...s, panes };
  });
}

export function startDeviceEditor(device?: Device, kind?: Device["kind"], folder?: string | null) {
  if (device) {
    setState({ deviceEditor: { ...device } });
    return;
  }
  const next = emptyDevice(kind ?? "ssh");
  next.folder = normalizeFolderPath(folder);
  setState({ deviceEditor: next });
}

rpc.onStatus((ok, err) => {
  setState({ daemonOk: ok, daemonError: ok ? null : err ?? "offline" });
});

rpc.onClosed((sessionId, reason) => {
  if (reconnectingSessions.has(sessionId)) return;
  if (userClosedSessions.has(sessionId) || reason === "closed") {
    userClosedSessions.delete(sessionId);
    return;
  }
  const pane = Object.values(getState().panes).find((p) => p.session?.id === sessionId);
  if (pane?.kind === "serial") {
    void reconnectPane(pane.id);
    return;
  }
  markDisconnected(sessionId, reason);
});

export function boot() {
  applyAppearance();
  void refreshAll();
  window.setInterval(() => {
    void rpc.health().then((ok) => {
      if (ok && !getState().daemonOk) void refreshAll();
      if (!ok) setState({ daemonOk: false, daemonError: getState().daemonError ?? "daemon unreachable" });
    });
  }, 4000);
}

