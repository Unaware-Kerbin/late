import { Component, useEffect, useRef, type ErrorInfo, type ReactNode } from "react";
import { ApiPane } from "./ApiPane";
import { PcapPane } from "./PcapPane";
import { SftpPane } from "./SftpPane";
import { StagePane } from "./StagePane";
import { TerminalPane } from "./TerminalPane";
import {
  closePaneSession,
  exportSession,
  sendBreak,
  setLogging,
  setSplitRatio,
  setState,
  splitKey,
  useApp,
} from "../store";
import type { PaneState, SplitNode } from "../types";

class PaneErrorBoundary extends Component<{ children: ReactNode }, { error: string | null }> {
  state = { error: null as string | null };
  static getDerivedStateFromError(err: Error) {
    return { error: err.message || String(err) };
  }
  componentDidCatch(err: Error, info: ErrorInfo) {
    console.error("pane crash", err, info.componentStack);
  }
  render() {
    if (this.state.error) {
      return (
        <div className="pcap-empty">
          This pane crashed: {this.state.error}
          <div className="meta">Try opening Packet capture again from the sidebar.</div>
        </div>
      );
    }
    return this.props.children;
  }
}

export function Workspace() {
  const tabs = useApp((s) => s.tabs);
  const activeTabId = useApp((s) => s.activeTabId);
  const panes = useApp((s) => s.panes);
  if (!tabs.length) return <div className="workspace empty">No workspace</div>;
  const active = tabs.find((t) => t.id === activeTabId) ?? tabs[0];
  // Only mount the visible tab. Hidden xterm canvases on Intel Arc crash
  // Electron's GPU process and paint the whole window black.
  return (
    <main className="workspace">
      <div key={active.id} className="workspace-layer" style={{ display: "flex" }}>
        <SplitView tabId={active.id} node={active.tree} panes={panes} visible />
      </div>
    </main>
  );
}

function SplitView({
  tabId,
  node,
  panes,
  visible,
}: {
  tabId: string;
  node: SplitNode;
  panes: Record<string, PaneState>;
  visible: boolean;
}) {
  if (node.type === "leaf") {
    const pane = panes[node.paneId];
    if (!pane) return <div className="pane empty">Missing pane</div>;
    return <PaneView pane={pane} visible={visible} />;
  }
  const dir = node.dir === "right" ? "row" : "column";
  return (
    <div className={`split ${node.dir}`}>
      <div style={{ flex: node.ratio, minWidth: 0, minHeight: 0, display: "flex" }}>
        <SplitView tabId={tabId} node={node.a} panes={panes} visible={visible} />
      </div>
      <SplitGutter
        dir={dir}
        onRatio={(r) => setSplitRatio(tabId, splitKey(node), r)}
      />
      <div style={{ flex: 1 - node.ratio, minWidth: 0, minHeight: 0, display: "flex" }}>
        <SplitView tabId={tabId} node={node.b} panes={panes} visible={visible} />
      </div>
    </div>
  );
}

function SplitGutter({ dir, onRatio }: { dir: string; onRatio: (r: number) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const down = (e: MouseEvent) => {
      e.preventDefault();
      const parent = el.parentElement;
      if (!parent) return;
      const rect = parent.getBoundingClientRect();
      const move = (ev: MouseEvent) => {
        const r =
          dir === "row" ? (ev.clientX - rect.left) / rect.width : (ev.clientY - rect.top) / rect.height;
        onRatio(r);
      };
      const up = () => {
        window.removeEventListener("mousemove", move);
        window.removeEventListener("mouseup", up);
      };
      window.addEventListener("mousemove", move);
      window.addEventListener("mouseup", up);
    };
    el.addEventListener("mousedown", down);
    return () => el.removeEventListener("mousedown", down);
  }, [dir, onRatio]);
  return <div ref={ref} className="gutter" />;
}

function PaneView({ pane, visible }: { pane: PaneState; visible: boolean }) {
  const focused = useApp((s) => s.focusedPaneId === pane.id);
  const devices = useApp((s) => s.inventory.devices);
  const accent =
    pane.session?.accent ?? devices.find((d) => d.id === pane.deviceId)?.accent ?? undefined;
  let body: JSX.Element;
  if (pane.kind === "sftp") body = <SftpPane pane={pane} />;
  else if (pane.kind === "pcap") body = <PcapPane pane={pane} />;
  else if (pane.kind === "api") body = <ApiPane pane={pane} />;
  else if (pane.kind === "stage") body = <StagePane key={`${pane.id}:${pane.stageId ?? ""}`} pane={pane} />;
  else if (pane.kind === "empty") {
    body = (
      <div className="empty">
        Select a device and press Enter, or open a local PTY from the sidebar.
      </div>
    );
  } else body = <TerminalPane pane={pane} visible={visible} />;

  return (
    <section
      className={`pane ${focused ? "focused" : ""}`}
      onMouseDown={() => setState({ focusedPaneId: pane.id })}
    >
      <div className="pane-bar" style={accent ? { boxShadow: `inset 3px 0 0 ${accent}` } : undefined}>
        <span className="kind-pill">{pane.kind}</span>
        <span className="pane-title">{pane.session?.name ?? pane.kind}</span>
        <span style={{ flex: 1 }} />
        {pane.session && pane.kind !== "pcap" && pane.kind !== "empty" && pane.kind !== "stage" && (
          <button
            type="button"
            className={`ghost ${pane.logging ? "on" : ""}`}
            title="Log this session to ~/.local/share/late/logs"
            onClick={(e) => {
              e.stopPropagation();
              void setLogging(pane.session!.id, !pane.logging);
            }}
          >
            {pane.logging ? "Logging" : "Log"}
          </button>
        )}
        {pane.session && (
          <button
            type="button"
            className="ghost"
            title="Export scrollback"
            onClick={(e) => {
              e.stopPropagation();
              void exportSession(pane.session!.id);
            }}
          >
            Export
          </button>
        )}
        {pane.kind === "serial" && pane.session && (
          <button
            type="button"
            className="ghost serial-break"
            disabled={pane.disconnected}
            title="Send a 350ms serial break (AOS-CX boot / menu interrupt)"
            onClick={(e) => {
              e.stopPropagation();
              void sendBreak(pane.session!.id);
            }}
          >
            Send Break
          </button>
        )}
        <button
          className="ghost"
          title="Close pane (reclaims split space)"
          onClick={() => void closePaneSession(pane.id)}
        >
          ×
        </button>
      </div>
      <PaneErrorBoundary key={pane.id}>{body}</PaneErrorBoundary>
    </section>
  );
}
