import { xtermTheme } from "../appearance";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef, useState } from "react";
import { rpc } from "../lib/rpc";
import { bumpTermFont, reconnectPane, resizeSession, sendBreak, sendInput, setState, useApp } from "../store";
import type { PaneState } from "../types";

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

  useEffect(() => {
    if (!host.current || !pane.session) return;
    const term = new Terminal({
      cursorBlink: true,
      fontFamily: "IBM Plex Mono, ui-monospace, monospace",
      fontSize: termFontSize,
      theme: xtermTheme(theme),
      scrollback: 50000,
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
    </>
  );
}

