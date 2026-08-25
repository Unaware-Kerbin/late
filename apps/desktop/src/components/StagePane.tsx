import { useEffect, useMemo, useState } from "react";
import { rpc } from "../lib/rpc";
import { openStagePane, toast, useApp } from "../store";
import type { Device, PaneState } from "../types";

const FORMATS = ["cli", "ansible", "netmiko", "salt", "chef"] as const;

function coerceFormat(v: unknown, fallback: (typeof FORMATS)[number] = "cli"): (typeof FORMATS)[number] {
  const s = String(v ?? "").trim().toLowerCase();
  return (FORMATS as readonly string[]).includes(s) ? (s as (typeof FORMATS)[number]) : fallback;
}

/** PATH Push talks SSH. A hostname/IP is enough even if the row was also used for serial. */
function isSshInventoryDevice(d: Device): boolean {
  const host = (d.host ?? "").trim();
  return !!host && !host.includes("://");
}

function pushLabel(format: (typeof FORMATS)[number]): string {
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

function confirmLabel(format: (typeof FORMATS)[number]): string {
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

type StageArt = {
  id?: string;
  format?: string;
  vendor?: string;
  intent?: string;
  body?: string;
  askOperator?: string;
  ask_operator?: string;
};

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

export function StagePane({ pane }: { pane: PaneState }) {
  const sessions = useApp((s) => s.sessions);
  const devices = useApp((s) => s.inventory.devices);
  const [format, setFormat] = useState<(typeof FORMATS)[number]>(() => coerceFormat(pane.stageFormat));
  const [intent, setIntent] = useState("");
  const [body, setBody] = useState("");
  const [ask, setAsk] = useState("");
  const [id, setId] = useState(pane.stageId ?? "");
  const [vendor, setVendor] = useState("");
  const [deviceId, setDeviceId] = useState(pane.deviceId ?? "");
  const [sessionId, setSessionId] = useState(pane.session?.id ?? "");
  const [preview, setPreview] = useState(false);
  const [busy, setBusy] = useState(false);
  const [plan, setPlan] = useState<StagePlan | null>(null);
  const [list, setList] = useState<{ id: string; format: string; intent: string }[]>([]);

  const pushable = useMemo(
    () => sessions.filter((s) => s.kind === "ssh" || s.kind === "serial"),
    [sessions],
  );
  const pathFormat = format !== "cli";
  const sshDevices = useMemo(() => devices.filter(isSshInventoryDevice), [devices]);
  const deviceChoices = pathFormat ? sshDevices : devices;

  useEffect(() => {
    if (!preview) return;
    const trap = (e: KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation();
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setPreview(false);
      }
    };
    window.addEventListener("keydown", trap, true);
    return () => window.removeEventListener("keydown", trap, true);
  }, [preview]);

  useEffect(() => {
    void refreshList();
  }, []);

  useEffect(() => {
    if (pane.stageFormat) setFormat(coerceFormat(pane.stageFormat));
  }, [pane.stageFormat]);

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

  function apply(art: StageArt) {
    setId(String(art.id ?? ""));
    setFormat(coerceFormat(art.format, "cli"));
    setIntent(art.intent ?? "");
    setBody(art.body ?? "");
    setVendor(art.vendor ?? "");
    setAsk(art.askOperator || art.ask_operator || "");
  }

  async function load(stageId: string) {
    try {
      const art = await rpc.call<StageArt>("stage.get", { id: stageId });
      apply(art);
    } catch (err) {
      toast("error", err instanceof Error ? err.message : String(err));
    }
  }

  function commonParams() {
    const path = format !== "cli";
    return {
      format,
      intent,
      body,
      id: id || undefined,
      deviceId: deviceId || undefined,
      device_id: deviceId || undefined,
      // PATH runners use inventory SSH. Push session is only for Push CLI into an open PTY.
      sessionId: path ? undefined : sessionId || undefined,
      session_id: path ? undefined : sessionId || undefined,
    };
  }

  async function renderDraft() {
    setBusy(true);
    try {
      const art = await rpc.call<StageArt>("stage.render", {
        format,
        intent,
        body: body.trim() ? body : undefined,
        deviceId: deviceId || undefined,
        device_id: deviceId || undefined,
        sessionId: format === "cli" ? sessionId || undefined : undefined,
        session_id: format === "cli" ? sessionId || undefined : undefined,
      });
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
      const art = await rpc.call<StageArt>("stage.save", commonParams());
      apply(art);
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
    if (format !== "cli") {
      const d = devices.find((x) => x.id === deviceId);
      if (!d || !isSshInventoryDevice(d)) {
        toast(
          "error",
          "Pick an SSH inventory device (hostname/IP) on your computer. Push session is only for Push CLI into an open terminal.",
        );
        return;
      }
    } else if (!sessionId) {
      toast("error", "Pick an open SSH or serial session for Push CLI.");
      return;
    }
    setBusy(true);
    try {
      const art = await rpc.call<StageArt>("stage.save", commonParams());
      apply(art);
      await refreshList();
      const next = await rpc.call<StagePlan>("stage.plan", {
        ...commonParams(),
        id: art.id || id,
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
      const result = await rpc.call<StagePushResult>("stage.push", commonParams(), 180_000);
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

  return (
    <div className="stage-pane">
      <p className="hint">
        Drafts stay on your computer. Ask the helper for an Ansible playbook, Netmiko script, Salt state, or Chef
        recipe and it will fill this pane. The helper cannot Push. Push CLI types into the <strong>Push session</strong>{" "}
        (open SSH/serial PTY). Ansible / Netmiko / Salt / Chef run on your computer with the <strong>Device</strong>{" "}
        inventory SSH host and that device's auth profile (key, agent, or the login Late already saved on your
        computer) — not the serial session. Late does not bundle those PATH tools.
      </p>
      <div className="stage-toolbar">
        <label>
          Format
          <select value={format} onChange={(e) => setFormat(e.target.value as (typeof FORMATS)[number])}>
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
      <textarea
        className="stage-body"
        spellCheck={false}
        value={body}
        onChange={(e) => setBody(e.target.value)}
        placeholder="CLI, playbook, Python, Salt, or Chef draft"
      />
      <div className="stage-toolbar">
        <button type="button" className="ghost" disabled={busy} onClick={() => void renderDraft()}>
          Render
        </button>
        <button type="button" className="ghost" disabled={busy} onClick={() => void saveDraft()}>
          Save draft
        </button>
        <button type="button" className="primary" disabled={busy || !body.trim()} onClick={() => void openPreview()}>
          {pushLabel(format)}
        </button>
      </div>
      {list.length > 0 && (
        <div className="stage-list">
          {list.map((row) => (
            <button
              key={row.id}
              type="button"
              className="ghost"
              onClick={() => {
                openStagePane(row.id, row.format);
                void load(row.id);
              }}
            >
              {row.format}: {row.intent || row.id.slice(0, 8)}
            </button>
          ))}
        </div>
      )}
      {preview && (
        <div className="modal-root" onMouseDown={() => setPreview(false)}>
          <div className="modal" onMouseDown={(e) => e.stopPropagation()}>
            <h2>{pushLabel(format).replace(/…$/, "?")}</h2>
            <p className="hint">
              {format === "cli"
                ? "This types the draft into the selected SSH/serial session. Click to confirm — Enter does not confirm."
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
    </div>
  );
}
