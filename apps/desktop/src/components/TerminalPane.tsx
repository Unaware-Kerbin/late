import { xtermTheme } from "../appearance";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef, useState } from "react";
import { clipboardRead, clipboardWrite } from "../lib/clipboard";
import { rpc } from "../lib/rpc";
import { bumpTermFont, reconnectPane, resizeSession, sendBreak, sendInput, setState, useApp } from "../store";
import type { PaneState } from "../types";

function isCopyKey(e: KeyboardEvent) {
  if (e.metaKey && !e.altKey && e.key.toLowerCase() === "c") return true;
  if (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === "c") return true;
  if (e.ctrlKey && e.key === "Insert") return true;
  return false;
}

function isPasteKey(e: KeyboardEvent) {
  if (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === "v") return true;
  if (e.shiftKey && !e.ctrlKey && !e.metaKey && e.key === "Insert") return true;
  return false;
}

function ptyPaste(text: string) {
  return text.replace(/\r\n/g, "\r").replace(/\n/g, "\r");
}

export function TerminalPane({ pane, visible = true }: { pane: PaneState; visible?: boolean }) {
  const host = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal>();
  const searchRef = useRef<SearchAddon>();
  const fitRef = useRef<FitAddon>();
  const termFontSize = useApp((s) => s.termFontSize);
  const theme = useApp((s) => s.theme);
  const activeTabId = useApp((s) => s.activeTabId);
  const focusedPaneId = useApp((s) => s.focusedPaneId);
  const [query, setQuery] = useState("");
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);

  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [menu]);

  useEffect(() => {
    if (!host.current || !pane.session) return;
    const term = new Terminal({
      cursorBlink: true,
      fontFamily: "ui-monospace, Cascadia Mono, SF Mono, Menlo, Consolas, monospace",
      fontSize: termFontSize,
      theme: xtermTheme(theme),
      scrollback: 50000,
      rightClickSelectsWord: false,
    });
    const fit = new FitAddon();
    const search = new SearchAddon();
    term.loadAddon(fit);
    term.loadAddon(search);
    // Canvas renderer: WebGL blanks the whole pane when the layout changes
    // (opening packet capture from a live local shell).
    term.open(host.current);
    const fitSoon = () => {
      requestAnimationFrame(() => {
        try {
          const el = host.current;
          if (!el || el.clientWidth < 8 || el.clientHeight < 8) return;
          fit.fit();
          if (pane.session) void resizeSession(pane.session.id, term.cols, term.rows);
        } catch {
          /* ignore */
        }
      });
    };
    fitSoon();
    void rpc
      .call<{ text?: string }>("session.scrollback", {
        id: pane.session.id,
        sessionId: pane.session.id,
        session_id: pane.session.id,
        redacted: false,
      })
      .then((r) => {
        const text = r?.text ?? "";
        if (text) term.write(text.replace(/\n/g, "\r\n"));
      })
      .catch(() => undefined);
    termRef.current = term;
    searchRef.current = search;
    fitRef.current = fit;

    const unsub = rpc.onBytes((sessionId, bytes) => {
      if (sessionId === pane.session?.id) term.write(bytes);
    });
    const onData = term.onData((data) => {
      if (pane.disconnected) {
        if (data === "\r") void reconnectPane(pane.id);
        return;
      }
      void sendInput(pane.session!.id, data);
    });
    const copySel = () => {
      const sel = term.getSelection();
      if (sel) void clipboardWrite(sel);
      return Boolean(sel);
    };
    const pasteClip = async () => {
      if (pane.disconnected || !pane.session) return;
      const text = await clipboardRead();
      if (text) void sendInput(pane.session.id, ptyPaste(text));
    };
    term.attachCustomKeyEventHandler((ev) => {
      if (ev.type !== "keydown") return true;
      if (isCopyKey(ev)) {
        copySel();
        return false;
      }
      if (isPasteKey(ev)) {
        void pasteClip();
        return false;
      }
      return true;
    });
    const onSel = term.onSelectionChange(() => {
      const sel = term.getSelection();
      if (sel) void clipboardWrite(sel, "selection");
    });
    const onContext = (e: MouseEvent) => {
      e.preventDefault();
      e.stopPropagation();
      setMenu({ x: e.clientX, y: e.clientY });
    };
    const onAux = (e: MouseEvent) => {
      if (e.button !== 1) return;
      e.preventDefault();
      void pasteClip();
    };
    term.element?.addEventListener("contextmenu", onContext);
    term.element?.addEventListener("auxclick", onAux);
    const ro = new ResizeObserver(() => fitSoon());
    ro.observe(host.current);
    window.addEventListener("resize", fitSoon);

    const onKey = (e: KeyboardEvent) => {
      if (
        pane.kind === "serial" &&
        pane.session &&
        e.ctrlKey &&
        (e.key === "Pause" || e.key === "Break")
      ) {
        e.preventDefault();
        void sendBreak(pane.session.id);
        return;
      }
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        setState((s) => ({
          ...s,
          panes: { ...s.panes, [pane.id]: { ...s.panes[pane.id], searchOpen: !s.panes[pane.id].searchOpen } },
        }));
      }
    };
    window.addEventListener("keydown", onKey);
    const onWheel = (e: WheelEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      e.preventDefault();
      bumpTermFont(e.deltaY < 0 ? 1 : -1);
    };
    host.current.addEventListener("wheel", onWheel, { passive: false });

    void rpc.call("session.resize", { sessionId: pane.session.id, cols: term.cols, rows: term.rows }).catch(() => undefined);

    return () => {
      unsub();
      onData.dispose();
      onSel.dispose();
      term.element?.removeEventListener("contextmenu", onContext);
      term.element?.removeEventListener("auxclick", onAux);
      ro.disconnect();
      window.removeEventListener("resize", fitSoon);
      window.removeEventListener("keydown", onKey);
      host.current?.removeEventListener("wheel", onWheel);
      term.dispose();
    };
  }, [pane.id, pane.session?.id, pane.disconnected]);

  useEffect(() => {
    if (!visible) return;
    const term = termRef.current;
    const fit = fitRef.current;
    const el = host.current;
    if (!term || !fit || !el || el.clientWidth < 8 || el.clientHeight < 8) return;
    term.options.fontSize = termFontSize;
    term.options.theme = xtermTheme(theme);
    try {
      fit.fit();
    } catch {
      /* ignore */
    }
    if (pane.session) {
      void resizeSession(pane.session.id, term.cols, term.rows);
    }
  }, [visible, termFontSize, theme, pane.session?.id, activeTabId, focusedPaneId]);

  return (
    <>
      {pane.kind === "serial" && pane.session && (
        <div className="serial-bar">
          <span>Serial console</span>
          <button
            type="button"
            className="ghost"
            disabled={pane.disconnected}
            title="Send a 350ms serial break (AOS-CX boot / menu interrupt). Ctrl+Break also works."
            onClick={() => void sendBreak(pane.session!.id)}
          >
            Send Break
          </button>
        </div>
      )}
      {pane.disconnected && (
        <div className="banner">
          {pane.disconnectReason ?? "Disconnected"} — press Enter to reconnect
        </div>
      )}
      {pane.searchOpen && (
        <div className="term-search">
          <input
            className="search"
            autoFocus
            placeholder="Find in buffer"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              if (e.target.value) searchRef.current?.findNext(e.target.value, { incremental: true, regex: false });
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                const opts = { incremental: false, regex: false, caseSensitive: false };
                if (e.shiftKey) searchRef.current?.findPrevious(query, opts);
                else searchRef.current?.findNext(query, opts);
              }
              if (e.key === "Escape") {
                setState((s) => ({
                  ...s,
                  panes: { ...s.panes, [pane.id]: { ...s.panes[pane.id], searchOpen: false } },
                }));
              }
            }}
          />
          <button
            type="button"
            className="ghost"
            title="Previous (Shift+Enter)"
            onClick={() => searchRef.current?.findPrevious(query, { incremental: false, regex: false })}
          >
            ↑
          </button>
          <button
            type="button"
            className="ghost"
            title="Next (Enter)"
            onClick={() => searchRef.current?.findNext(query, { incremental: false, regex: false })}
          >
            ↓
          </button>
          <span className="meta">Ctrl+F · Esc</span>
        </div>
      )}
      <div className="term-host" ref={host} />
      {menu && (
        <div
          className="ctx-menu"
          style={{ left: menu.x, top: menu.y }}
          onMouseDown={(e) => e.stopPropagation()}
        >
          <button
            type="button"
            disabled={!termRef.current?.hasSelection()}
            onClick={() => {
              const sel = termRef.current?.getSelection();
              if (sel) void clipboardWrite(sel);
              setMenu(null);
            }}
          >
            Copy
          </button>
          <button
            type="button"
            disabled={pane.disconnected || !pane.session}
            onClick={() => {
              setMenu(null);
              if (!pane.session || pane.disconnected) return;
              void clipboardRead().then((text) => {
                if (text && pane.session) void sendInput(pane.session.id, ptyPaste(text));
              });
            }}
          >
            Paste
          </button>
          <button
            type="button"
            onClick={() => {
              termRef.current?.selectAll();
              setMenu(null);
            }}
          >
            Select all
          </button>
        </div>
      )}
    </>
  );
}

