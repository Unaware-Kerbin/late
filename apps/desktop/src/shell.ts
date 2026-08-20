import type { LayoutId } from "./appearance";

export type ShellPaneId = "side" | "main" | "chat";

export const SHELL_DND = "text/plain";
export const SHELL_DND_PREFIX = "late-shell:";

export const SIDE_W = { min: 180, max: 560, fallback: 288 };
export const CHAT_W = { min: 280, max: 1600, fallback: 520 };
export const CHAT_H = { min: 200, max: 900, fallback: 400 };
export const MODELS_H = { min: 88, max: 720, fallback: 148 };

export function defaultOrder(layout: LayoutId): ShellPaneId[] {
  if (layout === "chat-left") return ["chat", "main", "side"];
  if (layout === "folders-right") return ["main", "chat", "side"];
  if (layout === "agent-top") return ["chat", "side", "main"];
  return ["side", "main", "chat"];
}

export function isAgentStacked(layout: LayoutId): boolean {
  return layout === "agent-top";
}

export function layoutFromOrder(order: ShellPaneId[]): LayoutId {
  const key = order.join(",");
  if (key === "chat,main,side") return "chat-left";
  if (key === "main,chat,side") return "folders-right";
  if (key === "side,main,chat") return "chat-right";
  return "custom";
}

export function clampChatHeight(px: number): number {
  const cap =
    typeof window !== "undefined" ? Math.round(window.innerHeight * 0.72) : CHAT_H.max;
  return Math.min(CHAT_H.max, cap, Math.max(CHAT_H.min, Math.round(px)));
}

export function clampModelsH(px: number): number {
  return Math.min(MODELS_H.max, Math.max(MODELS_H.min, Math.round(px)));
}

export function parseOrder(raw: string | null | undefined, fallback: ShellPaneId[]): ShellPaneId[] {
  const allowed = new Set<ShellPaneId>(["side", "main", "chat"]);
  const parts = (raw ?? "")
    .split(",")
    .map((p) => p.trim())
    .filter((p): p is ShellPaneId => allowed.has(p as ShellPaneId));
  if (parts.length !== 3 || new Set(parts).size !== 3) return fallback;
  return parts;
}

export function loadShellOrder(layout: LayoutId): ShellPaneId[] {
  const fallback = defaultOrder(layout);
  try {
    return parseOrder(localStorage.getItem("late.shellOrder"), fallback);
  } catch {
    return fallback;
  }
}

export function clampShellWidth(id: "side" | "chat", px: number): number {
  const b = id === "chat" ? CHAT_W : SIDE_W;
  let max = b.max;
  if (id === "chat" && typeof window !== "undefined") {
    max = Math.min(b.max, Math.max(b.min, window.innerWidth - SIDE_W.min - 180));
  }
  return Math.min(max, Math.max(b.min, Math.round(px)));
}

export function moveShellPane(order: ShellPaneId[], from: ShellPaneId, to: ShellPaneId): ShellPaneId[] {
  if (from === to) return order;
  const next = order.filter((id) => id !== from);
  const i = next.indexOf(to);
  if (i < 0) return order;
  next.splice(i, 0, from);
  return next;
}

export function encodeShellDrag(id: ShellPaneId): string {
  return `${SHELL_DND_PREFIX}${id}`;
}

export function decodeShellDrag(raw: string | null | undefined): ShellPaneId | null {
  const text = raw ?? "";
  if (!text.startsWith(SHELL_DND_PREFIX)) return null;
  const id = text.slice(SHELL_DND_PREFIX.length) as ShellPaneId;
  return id === "side" || id === "main" || id === "chat" ? id : null;
}

let dragging: ShellPaneId | null = null;

export function beginShellDrag(id: ShellPaneId) {
  dragging = id;
}

export function endShellDrag() {
  dragging = null;
}

export function draggingShell(): ShellPaneId | null {
  return dragging;
}
