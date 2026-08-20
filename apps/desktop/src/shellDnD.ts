import { beginShellDrag, encodeShellDrag, endShellDrag, SHELL_DND, type ShellPaneId } from "./shell";

export function onShellDragStart(e: { dataTransfer: DataTransfer }, id: ShellPaneId) {
  beginShellDrag(id);
  e.dataTransfer.setData(SHELL_DND, encodeShellDrag(id));
  e.dataTransfer.effectAllowed = "move";
}

export function onShellDragEnd() {
  endShellDrag();
}
