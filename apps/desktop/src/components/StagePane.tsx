import { Component, lazy, Suspense, useEffect, useMemo, useState, type ReactNode } from "react";
import { rpc } from "../lib/rpc";
import {
  isSshInventoryDevice,
  pushSessionChoices,
  readStageArt,
  resolveLivePushSession,
  resolvePathDeviceId,
  type StageArtFields,
} from "../lib/stageDrafts";
import { coerceStageFormat, STAGE_FORMATS, type StageFormat } from "../lib/stageEditorLang";
import { newStageDraft, openStagePane, toast, useApp } from "../store";
import type { PaneState } from "../types";

const FORMATS = STAGE_FORMATS;
const StageEditor = lazy(() => import("./StageEditor").then((m) => ({ default: m.StageEditor })));
const DRAFT_PLACEHOLDER = "CLI, playbook, Python, Salt, or Chef draft — Ctrl+Space for the right shape";

class StageEditorBoundary extends Component<{ children: ReactNode; fallback: ReactNode }, { bad: boolean }> {
  state = { bad: false };
  static getDerivedStateFromError() {
    return { bad: true };
  }
  render() {
    return this.state.bad ? this.props.fallback : this.props.children;
  }
}

function DraftFallback({
  value,
  onChange,
  placeholder,
}: {
  value: string;
  onChange: (next: string) => void;
  placeholder: string;
}) {
  return (
    <textarea
      className="stage-body"
      spellCheck={false}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      aria-label="Staging draft"
    />
  );
}

function coerceFormat(v: unknown, fallback: StageFormat = "cli"): StageFormat {
  return coerceStageFormat(v, fallback);
}

function pushLabel(format: StageFormat): string {
  switch (format) {
    case "cli":
      return "Push CLI…";
    case "ansible":
      return "Run playbook…";
    case "netmiko":
      return "Run script…";
    case "salt":
      return "Run state…";
    case "chef":
      return "Run recipe…";
  }
}

function confirmLabel(format: StageFormat): string {
  switch (format) {
    case "cli":
      return "Push";
    case "ansible":
      return "Run playbook";
    case "netmiko":
      return "Run script";
    case "salt":
      return "Run state";
    case "chef":
      return "Run recipe";
  }
}

type StagePlan = {
  format?: string;
  argv?: string[];
  display?: string;
  file?: string;
  binary?: string;
};

type StagePushResult = {
  ok?: boolean;
  display?: string;
  exitCode?: number;
  exit_code?: number;
  stdoutTail?: string;
  stdout_tail?: string;
  stderrTail?: string;
  stderr_tail?: string;
};

type DraftRow = { id: string; format: string; intent: string; vendor: string };

export function StagePane({ pane }: { pane: PaneState }) {
  const sessions = useApp((s) => s.sessions);
  const devices = useApp((s) => s.inventory.devices);
  const [format, setFormat] = useState<StageFormat>(() => coerceFormat(pane.stageFormat));
  const [intent, setIntent] = useState(() => pane.stageIntent ?? "");
  const [body, setBody] = useState(() => pane.stageBody ?? "");
  const [ask, setAsk] = useState("");
  const [id, setId] = useState(pane.stageId ?? "");
  const [vendor, setVendor] = useState("");
  const [deviceId, setDeviceId] = useState(pane.deviceId ?? "");
  const pushable = useMemo(() => pushSessionChoices(sessions), [sessions]);
  const [sessionId, setSessionId] = useState(() =>
    resolveLivePushSession(pane.stageSessionId || pane.session?.id || "", sessions),
  );
  const [preview, setPreview] = useState(false);
  const [busy, setBusy] = useState(false);
  const [plan, setPlan] = useState<StagePlan | null>(null);
  const [pushId, setPushId] = useState("");
  const [list, setList] = useState<DraftRow[]>([]);
  const [draftsOpen, setDraftsOpen] = useState(true);
  const [draftQuery, setDraftQuery] = useState("");
  const [deleteId, setDeleteId] = useState<string | null>(null);

  const pathFormat = format !== "cli";
  const sshDevices = useMemo(() => devices.filter(isSshInventoryDevice), [devices]);
  const deviceChoices = pathFormat ? sshDevices : devices;
  const visibleDrafts = useMemo(() => {
    const q = draftQuery.trim().toLowerCase();
    if (!q) return list;
    return list.filter((r) => `${r.format} ${r.intent} ${r.vendor} ${r.id}`.toLowerCase().includes(q));
  }, [list, draftQuery]);

  useEffect(() => {
    if (!preview && !deleteId) return;
    const trap = (e: KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation();
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setPreview(false);
        setDeleteId(null);
      }
    };
    window.addEventListener("keydown", trap, true);
    return () => window.removeEventListener("keydown", trap, true);
  }, [preview, deleteId]);

  useEffect(() => {
    void refreshList();
  }, []);

  useEffect(() => {
    if (pane.stageFormat) setFormat(coerceFormat(pane.stageFormat));
  }, [pane.stageFormat]);

  useEffect(() => {
    setSessionId((cur) => resolveLivePushSession(cur || pane.stageSessionId || "", pushable));
  }, [pushable, pane.stageSessionId]);

  useEffect(() => {
    if (!pane.stageId) return;
    void load(pane.stageId);
  }, [pane.stageId]);

  async function refreshList() {
    try {
      const rows = await rpc.call<unknown>("stage.list", {});
      const arr = Array.isArray(rows) ? rows : [];
      setList(
        arr
          .map((r) => {
            const o = r && typeof r === "object" ? (r as Record<string, unknown>) : {};
            return {
              id: String(o.id ?? ""),
              format: String(o.format ?? ""),
              intent: String(o.intent ?? ""),
              vendor: String(o.vendor ?? ""),
            };
          })
          .filter((r) => r.id),
      );
    } catch (err) {
      setList([]);
      const msg = err instanceof Error ? err.message : String(err);
      if (/unknown method|not found/i.test(msg)) {
        toast(
          "error",
          "This daemon build has no Staging RPCs. From this repo run npm start so it rebuilds late-daemon.",
        );
      }
    }
  }

  function apply(art: StageArtFields, opts?: { keepSeedBody?: boolean }) {
    setId(art.id);
    setFormat(art.format);
    setIntent(art.intent || pane.stageIntent || "");
    if (art.body.trim() || !opts?.keepSeedBody) setBody(art.body);
    else if (pane.stageBody) setBody(pane.stageBody);
    setVendor(art.vendor);
    setAsk(art.ask);
    const sess = resolveLivePushSession(art.sessionId || pane.stageSessionId || "", pushable);
    setSessionId(sess);
    const path = art.format !== "cli";
    const resolved = path
      ? resolvePathDeviceId(art.deviceId, sess || sessionId, sessions, devices)
      : art.deviceId;
    setDeviceId(resolved || art.deviceId || pane.deviceId || "");
  }

  async function load(stageId: string) {
    try {
      const art = readStageArt(await rpc.call<unknown>("stage.get", { id: stageId }));
      apply(art, { keepSeedBody: Boolean(pane.stageBody?.trim()) });
    } catch (err) {
      toast("error", err instanceof Error ? err.message : String(err));
    }
  }

  function commonParams(sessionOverride?: string) {
    const path = format !== "cli";
    const sess = (sessionOverride !== undefined ? sessionOverride : sessionId) || undefined;
    const device = path
      ? resolvePathDeviceId(deviceId, sessionId, sessions, devices) || deviceId || undefined
      : deviceId || undefined;
    return {
      format,
      intent,
      body,
      id: id || undefined,
      deviceId: device,
      device_id: device,
      // Keep session_id on PATH Push so late-core can recover the SSH inventory row
      // the helper saved. Serial sessions are still not the Ansible target.
      sessionId: sess,
      session_id: sess,
    };
  }

  async function renderDraft() {
    setBusy(true);
    try {
      const art = readStageArt(
        await rpc.call<unknown>("stage.render", {
          format,
          intent,
          body: body.trim() ? body : undefined,
          deviceId: deviceId || undefined,
          device_id: deviceId || undefined,
          sessionId: sessionId || undefined,
          session_id: sessionId || undefined,
        }),
      );
      apply(art);
    } catch (err) {
      toast("error", err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  async function saveDraft() {
    setBusy(true);
    try {
      const art = readStageArt(await rpc.call<unknown>("stage.save", commonParams()));
      apply(art);
      if (art.id) openStagePane(art.id, art.format);
      await refreshList();
      toast("ok", "draft saved on your computer (not sent to the device)");
    } catch (err) {
      toast("error", err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  async function openPreview() {
    if (!body.trim()) {
      toast("error", "Nothing to push.");
      return;
    }
    const liveSess = format === "cli" ? resolveLivePushSession(sessionId, pushable) : sessionId;
    if (liveSess && liveSess !== sessionId) setSessionId(liveSess);
    const pathDevice = pathFormat
      ? resolvePathDeviceId(deviceId, liveSess, sessions, devices) || deviceId
      : deviceId;
    if (format !== "cli") {
      const d = devices.find((x) => x.id === pathDevice);
      if (!d || !isSshInventoryDevice(d)) {
        toast(
          "error",
          "Pick an SSH inventory device (hostname/IP) on your computer. Push session is only for Push CLI into an open terminal.",
        );
        return;
      }
    } else if (!liveSess) {
      toast("error", "Pick an open SSH or serial session for Push CLI.");
      return;
    }
    setBusy(true);
    try {
      const art = readStageArt(await rpc.call<unknown>("stage.save", commonParams(liveSess)));
      apply(art);
      await refreshList();
      const savedId = art.id || id;
      setPushId(savedId);
      const next = await rpc.call<StagePlan>("stage.plan", {
        ...commonParams(liveSess),
        id: savedId || undefined,
        body: art.body || body,
        deviceId: pathDevice || undefined,
        device_id: pathDevice || undefined,
      });
      setPlan(next);
      setPreview(true);
    } catch (err) {
      toast("error", err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  async function confirmPush() {
    setBusy(true);
    try {
      const liveSess = format === "cli" ? resolveLivePushSession(sessionId, pushable) : sessionId;
      if (format === "cli" && !liveSess) {
        toast("error", "Pick an open SSH or serial session for Push CLI.");
        setBusy(false);
        return;
      }
      const pathDevice = pathFormat
        ? resolvePathDeviceId(deviceId, liveSess, sessions, devices) || deviceId
        : deviceId;
      const result = await rpc.call<StagePushResult>(
        "stage.push",
        {
          ...commonParams(liveSess),
          id: pushId || id || undefined,
          deviceId: pathDevice || undefined,
          device_id: pathDevice || undefined,
        },
        180_000,
      );
      setPreview(false);
      const tail = [result.stdoutTail ?? result.stdout_tail, result.stderrTail ?? result.stderr_tail]
        .filter((s) => typeof s === "string" && s.trim())
        .join("\n")
        .slice(0, 800);
      const headline = result.display || (result.ok === false ? "Push failed on your computer" : "Push finished on your computer");
      toast(result.ok === false ? "error" : "ok", tail ? `${headline}\n${tail}` : headline);
    } catch (err) {
      toast("error", err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  async function confirmDelete() {
    const target = deleteId;
    if (!target) return;
    setBusy(true);
    try {
      await rpc.call("stage.delete", { id: target });
      setDeleteId(null);
      if (id === target) {
        setId("");
        setIntent("");
        setBody("");
        setAsk("");
        setVendor("");
        setPushId("");
        newStageDraft();
      }
      await refreshList();
      toast("ok", "draft deleted on your computer");
    } catch (err) {
      toast("error", err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  const deleteRow = list.find((r) => r.id === deleteId);

  return (
    <div className="stage-pane">
      <p className="hint">
        Drafts stay on your computer. The editor follows Format (YAML, Python, Ruby, or CLI) and suggests the
        right shape — Ctrl+Space. Ask the helper for a playbook, script, state, or recipe and it will fill this
        pane. The helper cannot Push. Push CLI types into the <strong>Push session</strong>{" "}
        (open SSH/serial PTY). Ansible / Netmiko / Salt / Chef run on your computer with the <strong>Device</strong>{" "}
        inventory SSH host and that device&apos;s auth profile (key, agent, or the login Late already saved on your
        computer) — not the serial session. Late does not bundle those PATH tools.
      </p>
      <div className="stage-toolbar">
        <label>
          Format
          <select value={format} onChange={(e) => setFormat(e.target.value as StageFormat)}>
            {FORMATS.map((f) => (
              <option key={f} value={f}>
                {f}
              </option>
            ))}
          </select>
        </label>
        <label>
          {pathFormat ? "SSH device" : "Device"}
          <select value={deviceId} onChange={(e) => setDeviceId(e.target.value)}>
            <option value="">{pathFormat ? "(hostname/IP)" : "(inventory OS)"}</option>
            {deviceChoices.map((d) => (
              <option key={d.id} value={d.id}>
                {d.name}
                {d.host ? ` — ${d.host}` : ""} ({d.vendor})
              </option>
            ))}
          </select>
        </label>
        <label>
          {pathFormat ? "Push session (CLI only)" : "Push session"}
          <select value={sessionId} onChange={(e) => setSessionId(e.target.value)}>
            <option value="">(open SSH/serial)</option>
            {pushable.map((s) => (
              <option key={s.id} value={s.id}>
                {s.name}
              </option>
            ))}
          </select>
        </label>
        {vendor && <span className="kind-pill">{vendor}</span>}
      </div>
      <label className="stage-intent">
        Intent
        <input value={intent} onChange={(e) => setIntent(e.target.value)} placeholder="e.g. VLAN 10 on access ports" />
      </label>
      {ask && <p className="hint warn">{ask}</p>}
      <StageEditorBoundary
        fallback={<DraftFallback value={body} onChange={setBody} placeholder={DRAFT_PLACEHOLDER} />}
      >
        <Suspense fallback={<DraftFallback value={body} onChange={setBody} placeholder={DRAFT_PLACEHOLDER} />}>
          <StageEditor format={format} value={body} onChange={setBody} placeholder={DRAFT_PLACEHOLDER} />
        </Suspense>
      </StageEditorBoundary>
      <div className="stage-toolbar">
        <button type="button" className="ghost" disabled={busy} onClick={() => void renderDraft()}>
          Render
        </button>
        <button type="button" className="ghost" disabled={busy} onClick={() => void saveDraft()}>
          Save draft
        </button>
        <button type="button" className="ghost" disabled={busy} onClick={() => newStageDraft()}>
          New draft
        </button>
        <button type="button" className="primary" disabled={busy || !body.trim()} onClick={() => void openPreview()}>
          {pushLabel(format)}
        </button>
      </div>
      <section className={`stage-drafts${draftsOpen ? " open" : ""}`}>
        <div className="stage-drafts-head">
          <button
            type="button"
            className="stage-drafts-toggle"
            aria-expanded={draftsOpen}
            onClick={() => setDraftsOpen((v) => !v)}
          >
            <span className="pcap-chevron" aria-hidden>
              {draftsOpen ? "▾" : "▸"}
            </span>
            <span>Drafts on your computer</span>
            <span className="meta">{list.length ? `${list.length}` : "empty"}</span>
          </button>
        </div>
        {draftsOpen && (
          <div className="stage-drafts-body">
            <div className="stage-drafts-bar">
              <input
                value={draftQuery}
                onChange={(e) => setDraftQuery(e.target.value)}
                placeholder="Filter drafts…"
                aria-label="Filter drafts"
              />
              <button type="button" className="ghost" onClick={() => void refreshList()}>
                Refresh
              </button>
            </div>
            {visibleDrafts.length === 0 ? (
              <p className="hint">
                No saved drafts{draftQuery.trim() ? " match that filter" : " yet"}. Save draft keeps the file here —
                Push is still a separate click.
              </p>
            ) : (
              <ul className="stage-draft-list">
                {visibleDrafts.map((row) => (
                  <li key={row.id} className={row.id === id ? "sel" : ""}>
                    <button
                      type="button"
                      className="stage-draft-open"
                      onClick={() => openStagePane(row.id, row.format)}
                    >
                      <strong>
                        {row.format}
                        {row.vendor ? ` · ${row.vendor}` : ""}
                      </strong>
                      <span>{row.intent || row.id.slice(0, 8)}</span>
                    </button>
                    <button
                      type="button"
                      className="ghost tiny"
                      title="Delete draft"
                      onClick={() => setDeleteId(row.id)}
                    >
                      Delete
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
        )}
      </section>
      {preview && (
        <div className="modal-root" onMouseDown={() => setPreview(false)}>
          <div className="modal" onMouseDown={(e) => e.stopPropagation()}>
            <h2>{pushLabel(format).replace(/…$/, "?")}</h2>
            <p className="hint">
              {format === "cli"
                ? "This types the draft into the selected SSH/serial session. Permit list still runs. Click to confirm — Enter does not confirm."
                : "This runs the saved draft on your computer with the PATH tool shown below. Late does not bundle that tool. Click to confirm — Enter does not confirm."}
            </p>
            <pre className="stage-preview">{plan?.display || body.slice(0, 4000)}</pre>
            <div className="row">
              <button type="button" className="ghost" onClick={() => setPreview(false)}>
                Cancel
              </button>
              <button type="button" className="primary" disabled={busy} onClick={() => void confirmPush()}>
                {confirmLabel(format)}
              </button>
            </div>
          </div>
        </div>
      )}
      {deleteId && (
        <div className="modal-root" onMouseDown={() => setDeleteId(null)}>
          <div className="modal" onMouseDown={(e) => e.stopPropagation()}>
            <h2>Delete this draft?</h2>
            <p className="hint">
              Removes the file from Staging on your computer. It does not change the device.
              {deleteRow ? ` ${deleteRow.format}: ${deleteRow.intent || deleteRow.id.slice(0, 8)}` : ""}
            </p>
            <div className="row">
              <button type="button" className="ghost" onClick={() => setDeleteId(null)}>
                Cancel
              </button>
              <button type="button" className="primary" disabled={busy} onClick={() => void confirmDelete()}>
                Delete
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
