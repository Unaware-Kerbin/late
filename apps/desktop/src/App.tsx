import { useEffect, useState } from "react";
import { Modals } from "./components/Modals";
import { ShellLayout } from "./components/ShellLayout";
import { StatusBar } from "./components/StatusBar";
import {
  activateTab,
  boot,
  bumpTermFont,
  bumpUiScale,
  closeTab,
  getState,
  newTab,
  renameTab,
  resetAppearance,
  setState,
  splitFocused,
  openStagePane,
  useApp,
} from "./store";
import type { PaneState, SplitNode, SplitPlacement, TabState } from "./types";

const SPLIT_CHOICES: { place: SplitPlacement; label: string; hint: string }[] = [
  { place: "left", label: "Split left", hint: "Ctrl+Shift+←" },
  { place: "right", label: "Split right", hint: "Ctrl+Shift+→" },
  { place: "up", label: "Split up", hint: "Ctrl+Shift+↑" },
  { place: "down", label: "Split down", hint: "Ctrl+Shift+↓" },
];

function paneIdsIn(node: SplitNode): string[] {
  if (node.type === "leaf") return [node.paneId];
  return [...paneIdsIn(node.a), ...paneIdsIn(node.b)];
}

function tabSessionChrome(
  tab: TabState,
  panes: Record<string, PaneState>,
  devices: { id: string; accent?: string | null }[],
): { live: boolean; accent?: string } {
  for (const id of paneIdsIn(tab.tree)) {
    const pane = panes[id];
    if (!pane?.session) continue;
    const accent =
      pane.session.accent || devices.find((d) => d.id === pane.deviceId)?.accent || undefined;
    return { live: true, accent: accent || undefined };
  }
  return { live: false };
}

export function App() {
  const tabs = useApp((s) => s.tabs);
  const activeTabId = useApp((s) => s.activeTabId);
  const panes = useApp((s) => s.panes);
  const devices = useApp((s) => s.inventory.devices);
  const toasts = useApp((s) => s.toasts);
  const uiScale = useApp((s) => s.uiScale);
  const termFontSize = useApp((s) => s.termFontSize);
  const [tabMenu, setTabMenu] = useState<{ id: string; x: number; y: number } | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);

  useEffect(() => {
    boot();
    const onKey = (e: KeyboardEvent) => {
      const meta = e.ctrlKey || e.metaKey;
      if (meta && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setState((s) => ({ ...s, paletteOpen: true }));
      }
      if (meta && e.key === ",") {
        e.preventDefault();
        setState({ settingsOpen: true });
      }
      if (meta && e.key.toLowerCase() === "b") {
        e.preventDefault();
        setState((s) => ({ ...s, chatOpen: !s.chatOpen }));
      }
      if (meta && e.key.toLowerCase() === "n") {
        e.preventDefault();
        newTab();
      }
      if (meta && e.key.toLowerCase() === "w") {
        e.preventDefault();
        if (getState().activeTabId) closeTab(getState().activeTabId!);
      }
      if (meta && e.shiftKey && (e.key === "ArrowRight" || e.key.toLowerCase() === "r")) {
        e.preventDefault();
        splitFocused("right");
      }
      if (meta && e.shiftKey && (e.key === "ArrowLeft" || e.key.toLowerCase() === "l")) {
        e.preventDefault();
        splitFocused("left");
      }
      if (meta && e.shiftKey && (e.key === "ArrowDown" || e.key.toLowerCase() === "d")) {
        e.preventDefault();
        splitFocused("down");
      }
      if (meta && e.shiftKey && (e.key === "ArrowUp" || e.key.toLowerCase() === "u")) {
        e.preventDefault();
        splitFocused("up");
      }
      if (meta && (e.key === "=" || e.key === "+")) {
        e.preventDefault();
        bumpUiScale(0.1);
        bumpTermFont(1);
      }
      if (meta && e.key === "-") {
        e.preventDefault();
        bumpUiScale(-0.1);
        bumpTermFont(-1);
      }
      if (meta && e.key === "0") {
        e.preventDefault();
        resetAppearance();
      }
    };
    const onWheel = (e: WheelEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      e.preventDefault();
      const dir = e.deltaY < 0 ? 1 : -1;
      bumpUiScale(dir * 0.05);
      bumpTermFont(dir);
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("wheel", onWheel);
    };
  }, []);

  return (
    <div className="app">
      <header className="titlebar">
        <div className="brand">
          LATE <span>Local AI Terminal Emulator</span>
        </div>
        <div className="tabs" onClick={(e) => {
          if (e.button !== 0) return;
          setTabMenu(null);
        }}>
          {(tabs ?? []).map((t) => {
            const chrome = tabSessionChrome(t, panes, devices);
            return (
            <div
              key={t.id}
              className={`tab ${t.id === activeTabId ? "active" : ""} ${chrome.live ? "live" : ""}`}
              style={
                chrome.live
                  ? { ["--tab-accent" as string]: chrome.accent || "var(--accent)" }
                  : undefined
              }
              onDoubleClick={(e) => {
                e.stopPropagation();
                setRenaming(t.id);
              }}
              onContextMenu={(e) => {
                e.preventDefault();
                e.stopPropagation();
                setTabMenu({ id: t.id, x: e.clientX, y: e.clientY });
              }}
              onClick={(e) => {
                if (e.button !== 0) {
                  e.stopPropagation();
                  return;
                }
                activateTab(t.id);
                setTabMenu(null);
              }}
              onMouseDown={(e) => {
                if (e.button === 1) {
                  e.preventDefault();
                  closeTab(t.id);
                }
              }}
            >
              {renaming === t.id ? (
                <input
                  className="tab-rename"
                  defaultValue={t.title}
                  autoFocus
                  onClick={(e) => e.stopPropagation()}
                  onBlur={(e) => {
                    renameTab(t.id, e.target.value);
                    setRenaming(null);
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      renameTab(t.id, e.currentTarget.value);
                      setRenaming(null);
                    }
                    if (e.key === "Escape") setRenaming(null);
                  }}
                />
              ) : (
                <span className="tab-title">{t.title}</span>
              )}
              <button
                type="button"
                className="tab-close"
                title="Close tab"
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(t.id);
                }}
              >
                ×
              </button>
            </div>
            );
          })}
          <button className="icon-btn" onClick={() => newTab()} title="New tab">
            +
          </button>
        </div>
        <button
          className="icon-btn"
          title="Split left (Ctrl+Shift+←)"
          onClick={() => splitFocused("left")}
        >
          ◂
        </button>
        <button
          className="icon-btn"
          title="Split right (Ctrl+Shift+→)"
          onClick={() => splitFocused("right")}
        >
          ▸
        </button>
        <button
          className="icon-btn"
          title="Split up (Ctrl+Shift+↑)"
          onClick={() => splitFocused("up")}
        >
          ▴
        </button>
        <button
          className="icon-btn"
          title="Split down (Ctrl+Shift+↓)"
          onClick={() => splitFocused("down")}
        >
          ▾
        </button>
        <button className="icon-btn" onClick={() => setState((s) => ({ ...s, paletteOpen: true }))} title="Command palette">
          ⌘K
        </button>
        <button className="icon-btn" onClick={() => { bumpUiScale(-0.1); bumpTermFont(-1); }} title="Smaller text (Ctrl+-)">
          A−
        </button>
        <span className="zoom-readout" title="UI scale / terminal font">
          {Math.round(uiScale * 100)}% · {termFontSize}px
        </span>
        <button className="icon-btn" onClick={() => { bumpUiScale(0.1); bumpTermFont(1); }} title="Larger text (Ctrl+=)">
          A+
        </button>
        <button className="icon-btn" onClick={() => setState({ keysOpen: true })} title="API keys">
          Keys
        </button>
        <button className="icon-btn" onClick={() => openStagePane()} title="CLI / Ansible / Netmiko drafts">
          Staging
        </button>
        <button className="icon-btn" onClick={() => setState({ settingsOpen: true })} title="Settings, themes, layout">
          Settings
        </button>
        <button className="icon-btn" onClick={() => setState((s) => ({ ...s, chatOpen: !s.chatOpen }))} title="Toggle agent">
          Agent
        </button>
      </header>
      <ShellLayout />
      {tabMenu && (
        <div
          className="ctx-menu"
          style={{ left: tabMenu.x, top: tabMenu.y }}
          onMouseLeave={() => setTabMenu(null)}
        >
          <button
            type="button"
            onClick={() => {
              setRenaming(tabMenu.id);
              setTabMenu(null);
            }}
          >
            Rename tab
          </button>
          {SPLIT_CHOICES.map((c) => (
            <button
              key={c.place}
              type="button"
              onClick={() => {
                activateTab(tabMenu.id);
                splitFocused(c.place);
                setTabMenu(null);
              }}
            >
              {c.label}
            </button>
          ))}
          <button
            type="button"
            onClick={() => {
              closeTab(tabMenu.id);
              setTabMenu(null);
            }}
          >
            Close tab
          </button>
        </div>
      )}
      <StatusBar />
      <Modals />
      <div className="toasts">
        {(toasts ?? []).map((t) => (
          <div key={t.id} className={`toast ${t.kind}`}>
            {t.text}
          </div>
        ))}
      </div>
    </div>
  );
}
