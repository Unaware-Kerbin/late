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
