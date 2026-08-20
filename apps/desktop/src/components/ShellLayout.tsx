import { Fragment, useState, type CSSProperties, type DragEvent, type MouseEvent } from "react";
import { ChatPane } from "./ChatPane";
import { Sidebar } from "./Sidebar";
import { Workspace } from "./Workspace";
import { decodeShellDrag, draggingShell, endShellDrag, isAgentStacked, SHELL_DND, type ShellPaneId } from "../shell";
import { onShellDragEnd, onShellDragStart } from "../shellDnD";
import { dropShellPane, setChatHeight, setShellWidth, useApp } from "../store";

export function ShellLayout() {
  const chatOpen = useApp((s) => s.chatOpen);
  const layout = useApp((s) => s.layout);
  const order = useApp((s) => s.shellOrder);
  const sideW = useApp((s) => s.sideW);
  const chatW = useApp((s) => s.chatW);
  const chatH = useApp((s) => s.chatH);
  const [over, setOver] = useState<ShellPaneId | null>(null);
  const stacked = isAgentStacked(layout) && chatOpen;
  const rowIds = order.filter((id) => id !== "chat" || (!stacked && chatOpen));
  const slots = stacked ? rowIds.filter((id) => id !== "chat") : rowIds;

  function bindSlot(id: ShellPaneId) {
    return {
      className: `shell-slot ${id}${over === id ? " drop" : ""}`,
      onDragOver: (e: DragEvent) => {
        if (!draggingShell()) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
        setOver(id);
      },
      onDragLeave: (e: DragEvent) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node)) setOver((cur) => (cur === id ? null : cur));
      },
      onDrop: (e: DragEvent) => {
        e.preventDefault();
        const from = draggingShell() ?? decodeShellDrag(e.dataTransfer.getData(SHELL_DND));
        setOver(null);
        endShellDrag();
        if (from) dropShellPane(from, id);
      },
    };
  }

  const row = (
    <div className={stacked ? "body-row" : "body"}>
      {slots.map((id, i) => (
        <Fragment key={id}>
          {i > 0 && <ShellGutter axis="x" left={slots[i - 1]} right={id} />}
          <div {...bindSlot(id)} style={slotStyle(id, sideW, chatW, false)}>
            {id === "side" && <Sidebar />}
            {id === "main" && (
              <>
                <div
                  className="shell-head"
                  draggable
                  title="Drag to move the terminal pane"
                  onDragStart={(e) => onShellDragStart(e, "main")}
                  onDragEnd={onShellDragEnd}
                >
                  Terminal
                </div>
                <Workspace />
              </>
            )}
            {id === "chat" && <ChatPane />}
          </div>
        </Fragment>
      ))}
    </div>
  );

  if (!stacked) return row;

  return (
    <div className="body stacked">
      <div {...bindSlot("chat")} style={slotStyle("chat", sideW, chatW, true, chatH)}>
        <ChatPane />
      </div>
      <ShellGutter axis="y" />
      {row}
    </div>
  );
}

function slotStyle(
  id: ShellPaneId,
  sideW: number,
  chatW: number,
  stackedChat: boolean,
  chatH?: number,
): CSSProperties {
  if (id === "side") return { flex: `0 0 ${sideW}px`, width: sideW };
  if (id === "chat") {
    if (stackedChat) return { flex: `0 0 ${chatH}px`, height: chatH, width: "100%" };
    return { flex: `0 0 ${chatW}px`, width: chatW };
  }
  return { flex: "1 1 auto", minWidth: 160 };
}

function ShellGutter({
  axis,
  left,
  right,
}: {
  axis: "x" | "y";
  left?: ShellPaneId;
  right?: ShellPaneId;
}) {
  function onMouseDown(e: MouseEvent<HTMLDivElement>) {
    e.preventDefault();
    if (axis === "y") {
      const startY = e.clientY;
      const slot = e.currentTarget.previousElementSibling as HTMLElement | null;
      const startH = slot?.getBoundingClientRect().height ?? 280;
      const move = (ev: globalThis.MouseEvent) => setChatHeight(startH + (ev.clientY - startY));
      const up = () => {
        window.removeEventListener("mousemove", move);
        window.removeEventListener("mouseup", up);
      };
      window.addEventListener("mousemove", move);
      window.addEventListener("mouseup", up);
      return;
    }
    const gutter = e.currentTarget;
    const prev = gutter.previousElementSibling as HTMLElement | null;
    const next = gutter.nextElementSibling as HTMLElement | null;
    if (!prev || !next || !left || !right) return;
    const startX = e.clientX;
    const leftW = prev.getBoundingClientRect().width;
    const rightW = next.getBoundingClientRect().width;
    const leftFixed = left !== "main";
    const rightFixed = right !== "main";
    const move = (ev: globalThis.MouseEvent) => {
      const dx = ev.clientX - startX;
      if (leftFixed && rightFixed) {
        setShellWidth(left as "side" | "chat", leftW + dx);
        setShellWidth(right as "side" | "chat", rightW - dx);
      } else if (leftFixed) {
        setShellWidth(left as "side" | "chat", leftW + dx);
      } else if (rightFixed) {
        setShellWidth(right as "side" | "chat", rightW - dx);
      }
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  }
  return <div className={`shell-gutter ${axis === "y" ? "row" : "col"}`} onMouseDown={onMouseDown} />;
}
