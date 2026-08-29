import type { Device, SessionInfo } from "../types";
import { coerceStageFormat, type StageFormat } from "./stageEditorLang";

/** PATH Push talks SSH. A hostname/IP is enough even if the row was also used for serial. */
export function isSshInventoryDevice(d: Device): boolean {
  const host = (d.host ?? "").trim();
  return !!host && !host.includes("://");
}

export function strField(obj: object | null | undefined, keys: string[]): string {
  if (!obj) return "";
  const rec = obj as Record<string, unknown>;
  for (const k of keys) {
    const v = rec[k];
    if (typeof v === "string" && v.trim()) return v.trim();
  }
  return "";
}

export type StageArtFields = {
  id: string;
  format: StageFormat;
  intent: string;
  body: string;
  vendor: string;
  ask: string;
  deviceId: string;
  sessionId: string;
};

export type StageOpenSeed = {
  stageId?: string;
  format?: string;
  intent?: string;
  body?: string;
  sessionId?: string;
  deviceId?: string;
};

/** Tool-ok detail from propose_staged_artifact → fields the current Staging editor should show. */
export function stageOpenFromToolDetail(detail: unknown): StageOpenSeed {
  const art = readStageArt(detail);
  return {
    stageId: art.id || undefined,
    format: art.format,
    intent: art.intent || undefined,
    body: art.body.trim() ? art.body : undefined,
    sessionId: art.sessionId || undefined,
    deviceId: art.deviceId || undefined,
  };
}

/**
 * Remount Staging for a blank draft or a helper seed that carries editor fields.
 * Opening by id/format alone must not remount — Push CLI save used to bump epoch
 * on stageId and unmount the confirm modal before stage.push ran.
 */
export function stagingSeedRemounts(opts: {
  clear?: boolean;
  body?: string;
  intent?: string;
  sessionId?: string;
  deviceId?: string;
  stageId?: string;
  format?: string;
}): boolean {
  if (opts.clear) return true;
  return (
    opts.body !== undefined ||
    opts.intent !== undefined ||
    opts.sessionId !== undefined ||
    opts.deviceId !== undefined
  );
}

/** Opening a different draft by id must not keep the previous helper's editor seed. Same-id Push save keeps it. */
export function stageBodyAfterOpen(
  prev: { stageId?: string; stageBody?: string },
  opts: { clear?: boolean; stageId?: string; body?: string },
): string | undefined {
  if (opts.clear) return undefined;
  if (opts.body !== undefined) return opts.body;
  if (opts.stageId && opts.stageId !== prev.stageId) return undefined;
  return prev.stageBody;
}

/** Same-id Push save keeps the seed session. A different draft id must not Push the previous UUID. */
export function stageSessionAfterOpen(
  prev: { stageId?: string; stageSessionId?: string },
  opts: { clear?: boolean; stageId?: string; sessionId?: string },
): string | undefined {
  if (opts.clear) return undefined;
  if (opts.sessionId !== undefined) return opts.sessionId || undefined;
  if (opts.stageId && opts.stageId !== prev.stageId) return undefined;
  return prev.stageSessionId;
}

/** New draft keeps the inventory pick. Switching drafts drops a leftover helper device. */
export function stageDeviceAfterOpen(
  prev: { stageId?: string; deviceId?: string },
  opts: { clear?: boolean; stageId?: string; deviceId?: string },
): string | undefined {
  if (opts.clear) return prev.deviceId;
  if (opts.deviceId !== undefined) return opts.deviceId || undefined;
  if (opts.stageId && opts.stageId !== prev.stageId) return undefined;
  return prev.deviceId;
}

export function readStageArt(raw: unknown, fallbackFormat: StageFormat = "cli"): StageArtFields {
  const o = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  return {
    id: strField(o, ["id"]),
    format: coerceStageFormat(o.format, fallbackFormat),
    intent: strField(o, ["intent"]),
    body: typeof o.body === "string" ? o.body : "",
    vendor: strField(o, ["vendor"]),
    ask: strField(o, ["askOperator", "ask_operator"]),
    deviceId: strField(o, ["deviceId", "device_id"]),
    sessionId: strField(o, ["sessionId", "session_id"]),
  };
}

export function sessionDeviceId(s: SessionInfo | undefined): string {
  if (!s) return "";
  return strField(s as unknown as Record<string, unknown>, ["device_id", "deviceId"]);
}

/** PATH formats: use the saved SSH inventory row, else the open SSH session's device. Never a serial console. */
export function resolvePathDeviceId(
  savedDeviceId: string,
  sessionId: string,
  sessions: SessionInfo[],
  devices: Device[],
): string {
  if (savedDeviceId && devices.some((d) => d.id === savedDeviceId && isSshInventoryDevice(d))) {
    return savedDeviceId;
  }
  const sess = sessions.find((s) => s.id === sessionId);
  if (sess && String(sess.kind).toLowerCase() === "ssh") {
    const did = sessionDeviceId(sess);
    if (did && devices.some((d) => d.id === did && isSshInventoryDevice(d))) return did;
  }
  return savedDeviceId;
}

export function pushSessionChoices(sessions: SessionInfo[]): SessionInfo[] {
  return sessions.filter((s) => s.kind === "ssh" || s.kind === "serial");
}

/**
 * Push CLI must target a live SSH/serial PTY. A saved seed UUID that is not open
 * is ignored (daemon would 404). If the seed omitted session_id and exactly one
 * PTY is open, use it — same as local/cloud fill when the helper skipped the id.
 */
export function resolveLivePushSession(preferred: string, sessions: SessionInfo[]): string {
  const live = pushSessionChoices(sessions);
  const want = preferred.trim();
  if (want && live.some((s) => s.id === want)) return want;
  if (live.length === 1) return live[0].id;
  return "";
}
