import { folderPathIsUnder, normalizeFolderPath } from "../types";

export const LATE_DEVICE_MIME = "text/late-device";
export const LATE_FOLDER_MIME = "text/late-folder";
const DEVICE_PLAIN = "late-device:";
const FOLDER_PLAIN = "late-folder:";

export type SidebarDrag =
  | { kind: "device"; id: string }
  | { kind: "folder"; path: string };

export type SidebarDropTarget =
  | { type: "folder"; path: string }
  | { type: "device"; folder: string | null | undefined }
  | { type: "root" }
  | { type: "tools" };

/** Encode a sidebar inventory drag. `text/plain` is the Electron-safe fallback. */
export function writeSidebarDrag(dt: DataTransfer, drag: SidebarDrag): void {
  dt.effectAllowed = "move";
  if (drag.kind === "device") {
    dt.setData(LATE_DEVICE_MIME, drag.id);
    dt.setData("text/plain", `${DEVICE_PLAIN}${drag.id}`);
    return;
  }
  dt.setData(LATE_FOLDER_MIME, drag.path);
  dt.setData("text/plain", `${FOLDER_PLAIN}${drag.path}`);
}

export function readSidebarDrag(dt: DataTransfer): SidebarDrag | null {
  const device =
    dt.getData(LATE_DEVICE_MIME).trim() ||
    stripPrefix(dt.getData("text/plain"), DEVICE_PLAIN);
  if (device) return { kind: "device", id: device };
  const folder =
    normalizeFolderPath(dt.getData(LATE_FOLDER_MIME)) ||
    normalizeFolderPath(stripPrefix(dt.getData("text/plain"), FOLDER_PLAIN));
  if (folder) return { kind: "folder", path: folder };
  return null;
}

function stripPrefix(raw: string, prefix: string): string {
  const s = raw.trim();
  return s.startsWith(prefix) ? s.slice(prefix.length).trim() : "";
}

/** Folder path a dropped device should join. `null` = Sessions (ungrouped). */
export function deviceDropFolder(
  target: { type: "folder"; path: string } | { type: "device"; folder: string | null | undefined } | { type: "root" },
): string | null {
  if (target.type === "root") return null;
  if (target.type === "folder") return normalizeFolderPath(target.path);
  return normalizeFolderPath(target.folder);
}

/**
 * New path after dragging folder `from` onto parent `onto` (empty/null = Sessions).
 * `null` means the move is illegal (into self / descendant). Same path = no-op.
 */
export function folderDropDestination(from: string, onto: string | null | undefined): string | null {
  const src = normalizeFolderPath(from);
  if (!src) return null;
  const parent = normalizeFolderPath(onto);
  if ((parent ?? "") === src) return src;
  const leaf = src.includes("/") ? src.slice(src.lastIndexOf("/") + 1) : src;
  const dest = parent ? `${parent}/${leaf}` : leaf;
  if (dest === src) return src;
  if (folderPathIsUnder(dest, src) || (parent && folderPathIsUnder(parent, src))) return null;
  return dest;
}

export function sameDeviceFolder(current: string | null | undefined, next: string | null): boolean {
  return (normalizeFolderPath(current) ?? "") === (next ?? "");
}

/** Whether this row may accept the in-flight inventory drag (HTML5 dropEffect). */
export function sidebarDropAllowed(drag: SidebarDrag, target: SidebarDropTarget): boolean {
  if (target.type === "tools") return false;
  if (drag.kind === "device") return true;
  const onto =
    target.type === "root" ? null : target.type === "folder" ? target.path : deviceDropFolder(target);
  const dest = folderDropDestination(drag.path, onto);
  const src = normalizeFolderPath(drag.path);
  return Boolean(dest && dest !== src);
}
