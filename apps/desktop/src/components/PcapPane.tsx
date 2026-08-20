import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent as ReactMouseEvent } from "react";
import { clipboardWrite } from "../lib/clipboard";
import { rpc } from "../lib/rpc";
import { setState, toast, useApp } from "../store";
import type { PacketSummary, PaneState, PcapFinding, SessionInfo } from "../types";

type EdgesharkStatus = {
  running: boolean;
  starting: boolean;
  url: string;
  wireshark: boolean;
  plugin: boolean;
  detail: string;
};

type PcapOpen = {
  packets?: PacketSummary[];
  findings?: PcapFinding[];
  session?: SessionInfo;
  total?: number;
  truncated?: boolean;
  path?: string;
  file?: string;
  id?: string;
  captureId?: string;
  capture_id?: string;
  error?: string;
};

type SavedPcap = { name: string; path: string; bytes: number; mtime: number };

function strField(obj: object | null | undefined, keys: string[]): string {
  if (!obj) return "";
  const rec = obj as Record<string, unknown>;
  for (const k of keys) {
    const v = rec[k];
    if (typeof v === "string" && v.trim()) return v.trim();
    if (typeof v === "number" && Number.isFinite(v)) return String(v);
  }
  return "";
}

function captureIdOf(r: PcapOpen | null | undefined): string {
  return strField(r, ["id", "captureId", "capture_id"]);
}

function capturePathOf(r: PcapOpen | null | undefined): string {
  const direct = strField(r, ["path", "file"]);
  if (direct) return direct;
  if (r && typeof r.file === "object" && r.file) {
    return strField(r.file, ["path", "file"]);
  }
  return "";
}

function fmtBytes(n: number) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function fmtWhen(secs: number) {
  if (!secs) return "—";
  const d = new Date(secs * 1000);
  if (Number.isNaN(d.getTime())) return "—";
  return d.toLocaleString();
}

function shortPath(p: string) {
  if (!p) return "";
  const home = "/home/";
  if (p.includes("/.local/share/late/pcap/")) {
    return p.slice(p.lastIndexOf("/") + 1);
  }
  if (p.startsWith(home)) {
    const slash = p.indexOf("/", home.length);
    if (slash > 0) return `~${p.slice(slash)}`;
  }
  return p;
}

const KIND_LABEL: Record<string, string> = {
  retransmit: "Retransmits",
  zero_window: "Zero-window",
  dns_failure: "DNS failures",
  tls_alert: "TLS alerts",
  refused: "Refused / RST",
  icmp_unreach: "ICMP unreachable",
  arp_conflict: "ARP conflict",
  cleartext_creds: "Cleartext credentials",
};

function findingKind(f: PcapFinding) {
  const kind = f.kind || "finding";
  return KIND_LABEL[kind] || kind.replace(/_/g, " ");
}
function findingSummary(f: PcapFinding) {
  return f.summary || "";
}
function findingEvidence(f: PcapFinding) {
  return f.evidence || "";
}
function findingIndexes(f: PcapFinding) {
  return Array.isArray(f.packet_indexes) ? f.packet_indexes : [];
}

function rpcErr(err: unknown) {
  const msg = err instanceof Error ? err.message : String(err);
  if (/unknown method/i.test(msg)) {
    return "Daemon is stale (old process on :7420). Quit Late fully and run `late` again so it rebuilds and restarts the daemon.";
  }
  return msg;
}

function protoClass(proto: string) {
  const s = (proto || "").toLowerCase();
  if (s.includes("tcp")) return "proto-tcp";
  if (s.includes("udp")) return "proto-udp";
  if (s.includes("icmp")) return "proto-icmp";
  if (s.includes("arp")) return "proto-arp";
  if (s.includes("dns")) return "proto-dns";
  if (s.includes("tls") || s.includes("ssl") || s.includes("quic")) return "proto-tls";
  if (s.includes("http")) return "proto-http";
  return "proto-other";
}

function loadPref<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    if (raw == null) return fallback;
    return JSON.parse(raw) as T;
  } catch {
    return fallback;
  }
}

function savePref(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* ignore quota */
  }
}

export function PcapPane({ pane }: { pane: PaneState }) {
  const sessionId = pane.session?.id;
  const sessions = useApp((s) => s.sessions);
  const panes = useApp((s) => s.panes);
  const devices = useApp((s) => s.inventory?.devices);
  const sshSessions = [
    ...Object.values(panes ?? {})
      .map((p) => p.session)
      .filter((s): s is SessionInfo => !!s && String(s.kind).toLowerCase() === "ssh"),
    ...(sessions ?? []).filter((x) => String(x.kind).toLowerCase() === "ssh"),
  ].filter((s, i, all) => all.findIndex((x) => x.id === s.id) === i);
  const sshDevices = (devices ?? []).filter((d) => String(d.kind).toLowerCase() === "ssh");
  const [packets, setPackets] = useState<PacketSummary[]>([]);
  const [findings, setFindings] = useState<PcapFinding[]>([]);
  const [sel, setSel] = useState<PacketSummary | null>(null);
  const [activeFinding, setActiveFinding] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [quick, setQuick] = useState("");
  const [path, setPath] = useState("");
  const [iface, setIface] = useState("any");
  const [ifaces, setIfaces] = useState<string[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [localCaptureId, setLocalCaptureId] = useState<string | null>(null);
  const [sshCaptureId, setSshCaptureId] = useState<string | null>(null);
  const [savePath, setSavePath] = useState("");
  const [total, setTotal] = useState(0);
  const [truncated, setTruncated] = useState(false);
  const [edgeshark, setEdgeshark] = useState<EdgesharkStatus | null>(null);
  const [sshTarget, setSshTarget] = useState("");
  const [remoteIface, setRemoteIface] = useState("any");
  const [remoteCount, setRemoteCount] = useState("200");
  const [pcapDir, setPcapDir] = useState("");
  const [saved, setSaved] = useState<SavedPcap[]>([]);
  const [fileQuery, setFileQuery] = useState("");
  const [showMore, setShowMore] = useState(() => loadPref("late.pcap.showMore", false));
  const [showSide, setShowSide] = useState(() => loadPref("late.pcap.showSide", true));
  const [showDetail, setShowDetail] = useState(() => loadPref("late.pcap.showDetail", true));
  const [compact, setCompact] = useState(() => loadPref("late.pcap.compact", true));
  const [sideW, setSideW] = useState(() => loadPref("late.pcap.sideW", 280));
  const [detailH, setDetailH] = useState(() => loadPref("late.pcap.detailH", 160));
  const [captureOpen, setCaptureOpen] = useState(() => loadPref("late.pcap.captureOpen", true));
  const [savedOpen, setSavedOpen] = useState(() => loadPref("late.pcap.savedOpen", false));
  const paneRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => savePref("late.pcap.showMore", showMore), [showMore]);
  useEffect(() => savePref("late.pcap.showSide", showSide), [showSide]);
  useEffect(() => savePref("late.pcap.showDetail", showDetail), [showDetail]);
  useEffect(() => savePref("late.pcap.compact", compact), [compact]);
  useEffect(() => savePref("late.pcap.sideW", sideW), [sideW]);
  useEffect(() => savePref("late.pcap.detailH", detailH), [detailH]);
  useEffect(() => savePref("late.pcap.captureOpen", captureOpen), [captureOpen]);
  useEffect(() => savePref("late.pcap.savedOpen", savedOpen), [savedOpen]);
  useEffect(() => {
    if (savedOpen) void refreshSaved();
  }, [savedOpen]);

  async function refreshSaved() {
    try {
      const r = await rpc.call<{ dir?: string; files?: SavedPcap[] }>("pcap.list");
      setPcapDir(r.dir ?? "");
      setSaved(Array.isArray(r.files) ? r.files : []);
    } catch {
      /* daemon may still be coming up */
    }
  }

  useEffect(() => {
    void refreshEdgeshark();
    void refreshSaved();
    void rpc
      .call<string[]>("pcap.interfaces")
      .then((list) => {
        const nics = Array.isArray(list) ? list.filter(Boolean) : [];
        if (!nics.length) return;
        setIfaces(nics);
        setIface((cur) => {
          if (nics.includes("any") && (cur === "any" || !nics.includes(cur))) return "any";
          if (nics.includes(cur)) return cur;
          return nics.find((n) => n !== "lo" && !n.startsWith("br-") && n !== "docker0") ?? nics[0];
        });
      })
      .catch(() => undefined);
    const t = window.setInterval(() => void refreshEdgeshark(), 4000);
    return () => window.clearInterval(t);
  }, []);

  async function refreshEdgeshark() {
    try {
      const st = await rpc.call<EdgesharkStatus>("pcap.edgeshark.status");
      setEdgeshark(st);
    } catch {
      /* daemon may still be coming up */
    }
  }

  function applyOpen(r: PcapOpen, fallbackMsg?: string) {
    const list = (r.packets ?? []).slice(0, 400);
    setPackets(list);
    setFindings(r.findings ?? []);
    setSel(null);
    setActiveFinding(null);
    setTotal(r.total ?? list.length);
    setTruncated(!!r.truncated);
    const file = capturePathOf(r);
    if (file) {
      setPath(file);
      setSavePath(file);
    }
    void refreshSaved();
    setCaptureOpen(true);
    if (r.session) {
      setState((s) => ({
        panes: {
          ...s.panes,
          [pane.id]: {
            ...s.panes[pane.id],
            kind: "pcap",
            session: r.session,
            disconnected: false,
          },
        },
      }));
    }
    const shown = list.length;
    const all = r.total ?? shown;
    toast("ok", fallbackMsg ?? (r.truncated ? `showing ${shown} of ${all} packets` : `${shown} packets`));
  }

  async function finishCapture(r: PcapOpen, fallbackPath: string, doneMsg: string) {
    const file = capturePathOf(r) || fallbackPath;
    if ((!r.packets || r.packets.length === 0) && file) {
      const opened = await rpc.call<PcapOpen>("pcap.open", { path: file, file });
      applyOpen({ ...opened, path: capturePathOf(opened) || file, file }, doneMsg);
      return;
    }
    applyOpen({ ...r, path: file || r.path, file: file || r.file }, r.error ? String(r.error) : doneMsg);
  }

  async function openFile() {
    const file = path.trim();
    if (!file) {
      toast("error", "enter a .pcap path first");
      return;
    }
    setBusy("opening capture…");
    try {
      applyOpen(await rpc.call<PcapOpen>("pcap.open", { path: file, sessionId }));
    } catch (err) {
      toast("error", err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(null);
    }
  }

  async function live() {
    setBusy(`capturing on ${iface}…`);
    try {
      const cap = await rpc.call<PcapOpen>("pcap.start", {
        sessionId,
        iface,
        bpf: filter || undefined,
      });
      const id = captureIdOf(cap);
      if ((cap.packets?.length || cap.session) && !id) {
        applyOpen(cap, `local capture on ${iface}`);
        setLocalCaptureId(null);
      } else {
        const file = capturePathOf(cap);
        setSavePath(file);
        setLocalCaptureId(id || null);
        toast(
          "ok",
          file ? `capturing on ${iface} → ${file} — press Stop when done` : `capturing on ${iface} — press Stop when done`,
        );
        void refreshSaved();
      }
    } catch (err) {
      toast("error", err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(null);
    }
  }

  async function stopLive() {
    if (!localCaptureId) {
      toast("error", "No local capture is running.");
      return;
    }
    setBusy("stopping capture…");
    try {
      const r = await rpc.call<PcapOpen>(
        "pcap.stop",
        {
          id: localCaptureId,
          captureId: localCaptureId,
          capture_id: localCaptureId,
        },
        90_000,
      );
      const file = capturePathOf(r) || savePath;
      setLocalCaptureId(null);
      await finishCapture(r, file, "capture stopped");
    } catch (err) {
      toast("error", err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(null);
    }
  }

  async function applyFilter() {
    if (!sessionId) {
      toast("error", "open a capture first");
      return;
    }
    try {
      const r = await rpc.call<PacketSummary[]>("pcap.filter", { sessionId, expr: filter });
      setPackets(Array.isArray(r) ? r.slice(0, 400) : []);
      setSel(null);
    } catch (err) {
      toast("error", err instanceof Error ? err.message : String(err));
    }
  }

  function askAgent() {
    if (!sessionId && packets.length === 0) {
      toast("error", "open or capture packets first");
      return;
    }
    setState({
      chatOpen: true,
      agentSeed: {
        autoSend: true,
        text: `Analyze the open packet capture${sessionId ? ` (session ${sessionId})` : ""}. Use query_pcap for summary and findings (retransmits, zero-window, DNS failures, TLS alerts, ICMP unreach, refused/RST). Correlate with any open SSH or serial scrollback. What is going on, and what should I check next?`,
      },
    });
    toast("ok", "handed to the agent");
  }

  async function captureSsh() {
    if (!sshTarget) {
      toast("error", "open an SSH session or save an SSH device, then pick it here");
      return;
    }
    setBusy("starting SSH capture…");
    try {
      const isSession = sshSessions.some((s) => s.id === sshTarget);
      const device = sshDevices.find((d) => d.id === sshTarget);
      const authId = device?.auth_profile_id || undefined;
      const params = {
        sessionId: isSession ? sshTarget : undefined,
        session_id: isSession ? sshTarget : undefined,
        deviceId: isSession ? undefined : sshTarget,
        device_id: isSession ? undefined : sshTarget,
        auth_profile_id: isSession ? undefined : authId,
        authProfileId: isSession ? undefined : authId,
        iface: remoteIface || "any",
        bpf: filter || undefined,
      };
      let cap: PcapOpen;
      try {
        cap = await rpc.call<PcapOpen>("pcap.remote.start", params, 90_000);
      } catch {
        const r = await rpc.call<PcapOpen>(
          "pcap.remote",
          {
            ...params,
            count: Math.min(2000, Math.max(1, Number(remoteCount) || 200)),
          },
          90_000,
        );
        applyOpen(r, "SSH capture (one-shot)");
        return;
      }
      const id = captureIdOf(cap);
      const file = capturePathOf(cap);
      if ((cap.packets?.length || cap.session) && !id) {
        applyOpen(cap, file ? `SSH capture → ${file}` : "SSH capture");
        return;
      }
      if (!id) {
        toast("error", "SSH capture started but the daemon did not return an id");
        if (file) setSavePath(file);
        return;
      }
      setSavePath(file);
      setSshCaptureId(id);
      toast(
        "ok",
        file ? `SSH capture running → ${file} — press Stop when done` : "SSH capture running — press Stop when done",
      );
      void refreshSaved();
    } catch (err) {
      toast("error", err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(null);
    }
  }

  async function stopSsh() {
    if (!sshCaptureId) {
      toast("error", "No SSH capture is running.");
      return;
    }
    setBusy("stopping SSH capture…");
    try {
      const r = await rpc.call<PcapOpen>(
        "pcap.stop",
        {
          id: sshCaptureId,
          captureId: sshCaptureId,
          capture_id: sshCaptureId,
        },
        90_000,
      );
      const file = capturePathOf(r) || savePath;
      setSshCaptureId(null);
      await finishCapture(r, file, "SSH capture stopped");
    } catch (err) {
      toast("error", err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(null);
    }
  }

  async function startEdgeshark() {
    setBusy("starting Edgeshark…");
    try {
      const st = await rpc.call<EdgesharkStatus>("pcap.edgeshark.start", {}, 90_000);
      setEdgeshark(st);
      toast("ok", st.detail || "Edgeshark starting");
    } catch (err) {
      toast("error", rpcErr(err));
    } finally {
      setBusy(null);
    }
  }

  async function stopEdgeshark() {
    setBusy("stopping Edgeshark…");
    try {
      const st = await rpc.call<EdgesharkStatus>("pcap.edgeshark.stop");
      setEdgeshark(st);
      toast("ok", "Edgeshark stopped");
    } catch (err) {
      toast("error", err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(null);
    }
  }

  function openEdgesharkUi() {
    const url = edgeshark?.url || "http://127.0.0.1:5001";
    window.open(url, "_blank");
  }

  async function openWireshark() {
    try {
      await rpc.call("pcap.wireshark", { sessionId, path: path || savePath || undefined });
      toast("ok", "launched Wireshark");
    } catch (err) {
      toast("error", rpcErr(err));
    }
  }

  const fields = sel?.fields && typeof sel.fields === "object" ? Object.entries(sel.fields) : [];
  const es = edgeshark;
  const highlightIdx = new Set(findings.find((f) => f.id === activeFinding)?.packet_indexes ?? []);
  const openFilePath = savePath || path;
  const capturing = !!(localCaptureId || sshCaptureId);

  const visible = useMemo(() => {
    const q = quick.trim().toLowerCase();
    if (!q) return packets;
    return packets.filter((p) =>
      `${p.index} ${p.timestamp} ${p.src} ${p.dst} ${p.protocol} ${p.info} ${p.length}`.toLowerCase().includes(q),
    );
  }, [packets, quick]);

  const savedVisible = useMemo(() => {
    const q = fileQuery.trim().toLowerCase();
    const list = q ? saved.filter((f) => f.name.toLowerCase().includes(q) || f.path.toLowerCase().includes(q)) : saved;
    return list;
  }, [saved, fileQuery]);

  function openSaved(f: SavedPcap) {
    setPath(f.path);
    setBusy("opening capture…");
    void rpc
      .call<PcapOpen>("pcap.open", { path: f.path })
      .then((r) => applyOpen(r, f.name))
      .catch((err) => toast("error", err instanceof Error ? err.message : String(err)))
      .finally(() => setBusy(null));
  }

  function jumpFinding(f: PcapFinding) {
    const next = activeFinding === f.id ? null : f.id;
    setActiveFinding(next);
    if (!next) return;
    const idx = findingIndexes(f)[0];
    if (idx == null) return;
    const pkt = packets.find((p) => p.index === idx);
    if (pkt) setSel(pkt);
    requestAnimationFrame(() => {
      listRef.current?.querySelector(`[data-pcap-idx="${idx}"]`)?.scrollIntoView({ block: "nearest" });
    });
  }

  const selectAt = useCallback(
    (indexInVisible: number) => {
      const pkt = visible[indexInVisible];
      if (!pkt) return;
      setSel(pkt);
      requestAnimationFrame(() => {
        listRef.current?.querySelector(`[data-pcap-idx="${pkt.index}"]`)?.scrollIntoView({ block: "nearest" });
      });
    },
    [visible],
  );

  function onPaneKey(e: KeyboardEvent<HTMLDivElement>) {
    const tag = (e.target as HTMLElement)?.tagName;
    if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") return;
    if (!visible.length) return;
    const cur = sel ? visible.findIndex((p) => p.index === sel.index) : -1;
    if (e.key === "ArrowDown" || e.key === "j") {
      e.preventDefault();
      selectAt(Math.min(visible.length - 1, (cur < 0 ? -1 : cur) + 1));
    } else if (e.key === "ArrowUp" || e.key === "k") {
      e.preventDefault();
      selectAt(Math.max(0, (cur < 0 ? 0 : cur) - 1));
    } else if (e.key === "Home") {
      e.preventDefault();
      selectAt(0);
    } else if (e.key === "End") {
      e.preventDefault();
      selectAt(visible.length - 1);
    } else if (e.key === "Escape") {
      setSel(null);
      setActiveFinding(null);
    }
  }

  function startDrag(axis: "x" | "y", invert: boolean, value: number, set: (n: number) => void, min: number, max: number) {
    return (e: ReactMouseEvent<HTMLDivElement>) => {
      e.preventDefault();
      const start = axis === "x" ? e.clientX : e.clientY;
      const startVal = value;
      const move = (ev: globalThis.MouseEvent) => {
        const d = (axis === "x" ? ev.clientX : ev.clientY) - start;
        const next = invert ? startVal - d : startVal + d;
        set(Math.round(Math.min(max, Math.max(min, next))));
      };
      const up = () => {
        window.removeEventListener("mousemove", move);
        window.removeEventListener("mouseup", up);
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
      };
      document.body.style.cursor = axis === "x" ? "col-resize" : "row-resize";
      document.body.style.userSelect = "none";
      window.addEventListener("mousemove", move);
      window.addEventListener("mouseup", up);
    };
  }

  function copySel() {
    if (!sel) return;
    const text = `#${sel.index} ${sel.timestamp} ${sel.src} → ${sel.dst} ${sel.protocol} ${sel.info}`;
    void clipboardWrite(text).then((ok) => {
      if (ok) toast("ok", "copied packet line");
      else toast("error", "could not copy");
    });
  }

  const statusBits: string[] = [];
  if (busy) statusBits.push(busy);
  else if (sshCaptureId) statusBits.push("SSH capturing — Stop when done");
  else if (localCaptureId) statusBits.push(`Live on ${iface} — Stop when done`);
  if (truncated) statusBits.push(`showing ${packets.length} of ${total}`);
  else if (packets.length) statusBits.push(`${visible.length === packets.length ? packets.length : `${visible.length}/${packets.length}`} pkts`);
  if (sel) statusBits.push(`#${sel.index}`);
  if (quick.trim()) statusBits.push(`find “${quick.trim()}”`);
  if (filter.trim()) statusBits.push(`bpf ${filter.trim()}`);
  if (openFilePath) statusBits.push(shortPath(openFilePath));

  return (
    <div
      className={`pcap-pane${compact ? " compact" : ""}${capturing ? " capturing" : ""}`}
      ref={paneRef}
      tabIndex={0}
      onKeyDown={onPaneKey}
    >
      <section className={`pcap-folder${captureOpen ? " open" : ""}`}>
        <div className="pcap-folder-head">
          <button
            type="button"
            className="pcap-folder-toggle"
            aria-expanded={captureOpen}
            onClick={() => setCaptureOpen((v) => !v)}
          >
            <span className="pcap-chevron" aria-hidden>
              {captureOpen ? "▾" : "▸"}
            </span>
            <span>Capture</span>
            <span className="meta">
              {capturing ? "live" : packets.length ? `${packets.length} pkts` : "idle"}
            </span>
          </button>
          <button
            type="button"
            className="pcap-folder-close"
            title="Close Capture"
            disabled={!captureOpen}
            onClick={() => setCaptureOpen(false)}
          >
            ×
          </button>
        </div>
        {captureOpen && (
          <>
      <div className="pcap-toolbar pcap-mainbar">
        <span className="pcap-label">Local</span>
        {ifaces.length ? (
          <select className="pcap-input" value={iface} onChange={(e) => setIface(e.target.value)} title="Local interface">
            {ifaces.map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        ) : (
          <input
            className="pcap-input"
            placeholder="iface"
            value={iface}
            onChange={(e) => setIface(e.target.value)}
          />
        )}
        {localCaptureId ? (
          <button type="button" className="danger" onClick={() => void stopLive()}>
            Stop live
          </button>
        ) : (
          <button type="button" className="primary" onClick={() => void live()} disabled={!!sshCaptureId}>
            Live
          </button>
        )}
        <span className="pcap-sep" />
        <span className="pcap-label">SSH</span>
        <select
          className="pcap-input"
          value={sshTarget}
          onChange={(e) => setSshTarget(e.target.value)}
          title="SSH session or saved device"
        >
          <option value="">{sshSessions.length || sshDevices.length ? "session or device…" : "open SSH first…"}</option>
          {sshSessions.map((s) => (
            <option key={s.id} value={s.id}>
              session: {s.name}
            </option>
          ))}
          {sshDevices.map((d) => (
            <option key={d.id} value={d.id}>
              device: {d.name}
              {d.host ? ` (${d.host})` : ""}
            </option>
          ))}
        </select>
        <input
          className="pcap-input narrow"
          placeholder="iface"
          value={remoteIface}
          onChange={(e) => setRemoteIface(e.target.value)}
          title="Remote interface"
        />
        {sshCaptureId ? (
          <button type="button" className="danger" onClick={() => void stopSsh()}>
            Stop SSH
          </button>
        ) : (
          <button type="button" className="primary" disabled={!!localCaptureId} onClick={() => void captureSsh()}>
            Capture
          </button>
        )}
        <span className="pcap-sep" />
        <input
          className="pcap-input wide"
          placeholder="Find in this list…"
          value={quick}
          onChange={(e) => setQuick(e.target.value)}
          title="Instant display filter — does not change the capture"
        />
        <button type="button" className="ghost" onClick={() => askAgent()} disabled={!sessionId && packets.length === 0}>
          Ask agent
        </button>
        <button type="button" className={`ghost${showMore ? " on" : ""}`} onClick={() => setShowMore((v) => !v)}>
          More
        </button>
      </div>
      {showMore && (
        <div className="pcap-toolbar pcap-more">
          <span className="pcap-label">File</span>
          <input
            className="pcap-input wide"
            placeholder="/path/to/file.pcap"
            value={path}
            onChange={(e) => setPath(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void openFile();
            }}
          />
          <button type="button" className="ghost" onClick={() => void openFile()}>
            Open
          </button>
          <button type="button" className="ghost" onClick={() => void openWireshark()}>
            Wireshark
          </button>
          <span className="pcap-sep" />
          <span className="pcap-label">BPF</span>
          <input
            className="pcap-input wide"
            placeholder="host 10.0.0.1 or port 53"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void applyFilter();
            }}
            title="Applied on Live/SSH start, or Apply on an open capture"
          />
          <button type="button" className="ghost" onClick={() => void applyFilter()}>
            Apply
          </button>
          <span className="pcap-sep" />
          <span className="pcap-label">SSH count</span>
          <input
            className="pcap-input narrow"
            placeholder="200"
            value={remoteCount}
            onChange={(e) => setRemoteCount(e.target.value)}
            title="One-shot packet count if live start is unavailable"
          />
          <span className="pcap-sep" />
          <span className="pcap-label">Edgeshark</span>
          <span className="pcap-hint">
            {es?.starting ? "starting…" : es?.running ? "running" : "Docker containers"}
            {es && !es.plugin ? " · needs cshargextcap" : ""}
          </span>
          <button type="button" className="ghost" onClick={() => void startEdgeshark()}>
            Start
          </button>
          <button type="button" className="ghost" onClick={openEdgesharkUi}>
            UI
          </button>
          <button type="button" className="ghost" onClick={() => void stopEdgeshark()}>
            Stop
          </button>
        </div>
      )}
      <div className="pcap-statusbar">
        <span className={capturing ? "pcap-dot live" : "pcap-dot"} />
        <span className="pcap-status-text">{statusBits.join(" · ") || "No capture open"}</span>
        <span className="pcap-status-spacer" />
        {captureOpen && (
          <>
            <button type="button" className={`ghost tiny${compact ? " on" : ""}`} onClick={() => setCompact((v) => !v)}>
              Compact
            </button>
            <button type="button" className={`ghost tiny${showDetail ? " on" : ""}`} onClick={() => setShowDetail((v) => !v)}>
              Detail
            </button>
            <button type="button" className={`ghost tiny${showSide ? " on" : ""}`} onClick={() => setShowSide((v) => !v)}>
              Findings
            </button>
          </>
        )}
      </div>
      <div className="pcap-workspace">
        <div className="pcap-center">
          <div className="pcap-list" ref={listRef}>
            {packets.length === 0 ? (
              <div className="pcap-empty">
                <div className="pcap-cards">
                  <button type="button" className="pcap-card" onClick={() => void live()} disabled={!!sshCaptureId}>
                    <strong>Local live</strong>
                    <span>dumpcap on {iface || "any"}. Stop from the toolbar when you have enough traffic.</span>
                  </button>
                  <button type="button" className="pcap-card" onClick={() => void captureSsh()} disabled={!sshTarget || !!localCaptureId}>
                    <strong>SSH capture</strong>
                    <span>
                      {sshTarget
                        ? "Second SSH to the device — your terminal stays open. Stop when done."
                        : "Pick a session or saved SSH device in the toolbar, then start."}
                    </span>
                  </button>
                  <button type="button" className="pcap-card" onClick={() => setSavedOpen(true)}>
                    <strong>Saved files</strong>
                    <span>
                      {saved.length
                        ? `${saved.length} capture${saved.length === 1 ? "" : "s"} in ${pcapDir || "~/.local/share/late/pcap"}`
                        : `Past captures will show up in the Saved files tab (${pcapDir || "~/.local/share/late/pcap"}).`}
                    </span>
                  </button>
                </div>
                <p className="pcap-empty-hint">
                  Drag the gutters to resize the list, packet fields, and sidebar. Arrow keys move the selected packet.
                  Display filter is instant; Apply sends BPF to the daemon.
                </p>
              </div>
            ) : (
              <table className="pcap-table">
                <thead>
                  <tr>
                    <th className="col-n">#</th>
                    <th className="col-time">Time</th>
                    <th>Src</th>
                    <th>Dst</th>
                    <th className="col-proto">Proto</th>
                    <th className="col-len">Len</th>
                    <th>Info</th>
                  </tr>
                </thead>
                <tbody>
                  {visible.map((p, i) => (
                    <tr
                      key={`${p.index ?? i}`}
                      data-pcap-idx={p.index}
                      className={[
                        sel?.index === p.index ? "sel" : "",
                        highlightIdx.has(p.index) ? "hit" : "",
                      ]
                        .filter(Boolean)
                        .join(" ")}
                      onClick={() => {
                        setSel(p);
                        paneRef.current?.focus();
                      }}
                      onDoubleClick={copySel}
                    >
                      <td className="col-n">{p.index}</td>
                      <td className="col-time">{p.timestamp}</td>
                      <td>{p.src}</td>
                      <td>{p.dst}</td>
                      <td className="col-proto">
                        <span className={`pcap-proto ${protoClass(p.protocol)}`}>{p.protocol || "—"}</span>
                      </td>
                      <td className="col-len">{p.length || ""}</td>
                      <td className="col-info" title={p.info}>
                        {p.info}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
          {showDetail && (
            <>
              <div
                className="gutter pcap-gutter-h"
                title="Drag to resize packet detail"
                onMouseDown={startDrag("y", true, detailH, setDetailH, 72, 420)}
              />
              <div className="pcap-detail" style={{ height: detailH }}>
                <div className="pcap-detail-head">
                  <h4>Packet fields</h4>
                  {sel ? (
                    <span className="meta">
                      #{sel.index} · {sel.src} → {sel.dst} · {sel.protocol}
                      <button type="button" className="linkish" onClick={copySel}>
                        copy
                      </button>
                    </span>
                  ) : (
                    <span className="meta">Select a packet, or use ↑ ↓</span>
                  )}
                </div>
                {sel ? (
                  fields.length ? (
                    <div className="pcap-kv-grid">
                      {fields.map(([k, v]) => (
                        <div key={k} className="pcap-kv">
                          <span>{k}</span>
                          <span>{String(v)}</span>
                        </div>
                      ))}
                    </div>
                  ) : (
                    <p className="meta pad">No dissected fields on this packet.</p>
                  )
                ) : (
                  <p className="meta pad">Nothing selected.</p>
                )}
              </div>
            </>
          )}
        </div>
        {showSide && (
          <>
            <div
              className="gutter pcap-gutter-v"
              title="Drag to resize sidebar"
              onMouseDown={startDrag("x", true, sideW, setSideW, 180, 560)}
            />
            <aside className="pcap-side" style={{ width: sideW }}>
              <section className="pcap-side-sec">
                <div className="pcap-side-head">
                  <h4>Findings{findings.length ? ` (${findings.length})` : ""}</h4>
                </div>
                {findings.length === 0 ? (
                  <p className="meta">
                    {packets.length === 0
                      ? "Open or stop a capture to compute findings."
                      : "No retransmits, DNS failures, TLS alerts, ICMP unreachables, or refused connections."}
                  </p>
                ) : (
                  findings.map((f) => (
                    <button
                      key={f.id}
                      type="button"
                      className={activeFinding === f.id ? "pcap-finding active" : "pcap-finding"}
                      onClick={() => jumpFinding(f)}
                    >
                      <strong className="pcap-finding-kind">{findingKind(f)}</strong> {findingSummary(f)}
                      {findingEvidence(f) ? <div className="meta">{findingEvidence(f)}</div> : null}
                      {findingIndexes(f).length ? (
                        <div className="meta">pkts {findingIndexes(f).slice(0, 8).join(", ")}</div>
                      ) : null}
                    </button>
                  ))
                )}
              </section>
            </aside>
          </>
        )}
      </div>
          </>
        )}
      </section>
      <section className={`pcap-folder${savedOpen ? " open" : ""}`}>
        <div className="pcap-folder-head">
          <button
            type="button"
            className="pcap-folder-toggle"
            aria-expanded={savedOpen}
            onClick={() => setSavedOpen((v) => !v)}
          >
            <span className="pcap-chevron" aria-hidden>
              {savedOpen ? "▾" : "▸"}
            </span>
            <span>Saved files</span>
            <span className="meta">{saved.length ? `${saved.length}` : "empty"}</span>
          </button>
          <button
            type="button"
            className="pcap-folder-close"
            title="Close Saved files"
            disabled={!savedOpen}
            onClick={() => setSavedOpen(false)}
          >
            ×
          </button>
        </div>
        {savedOpen && (
          <div className="pcap-library">
            <div className="pcap-library-bar">
              <input
                className="pcap-input wide"
                placeholder="Filter saved captures…"
                value={fileQuery}
                onChange={(e) => setFileQuery(e.target.value)}
              />
              <button type="button" className="ghost" onClick={() => void refreshSaved()}>
                Refresh
              </button>
              <span className="meta">{pcapDir || "~/.local/share/late/pcap"}</span>
            </div>
            {savedVisible.length === 0 ? (
              <div className="pcap-empty">
                <p>
                  No saved captures{fileQuery.trim() ? " match that filter" : " yet"}. Start <strong>Live</strong> or{" "}
                  <strong>SSH Capture</strong>, then Stop — the file lands here.
                </p>
              </div>
            ) : (
              <table className="pcap-table pcap-lib-table">
                <thead>
                  <tr>
                    <th>Name</th>
                    <th className="col-len">Size</th>
                    <th className="col-time">Modified</th>
                    <th className="col-open"> </th>
                  </tr>
                </thead>
                <tbody>
                  {savedVisible.map((f) => (
                    <tr
                      key={f.path}
                      className={openFilePath === f.path ? "sel" : ""}
                      title={f.path}
                      onClick={() => openSaved(f)}
                    >
                      <td>{f.name}</td>
                      <td className="col-len">{fmtBytes(f.bytes)}</td>
                      <td className="col-time">{fmtWhen(f.mtime)}</td>
                      <td className="col-open">
                        <button
                          type="button"
                          className="ghost tiny"
                          onClick={(e) => {
                            e.stopPropagation();
                            openSaved(f);
                          }}
                        >
                          Open
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
        )}
      </section>
    </div>
  );
}
