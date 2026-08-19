import { useEffect, useMemo, useState } from "react";
import { rpc } from "../lib/rpc";
import { toast, useApp } from "../store";
import type { PaneState } from "../types";

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE"] as const;

function asRecord(v: unknown): Record<string, unknown> | null {
  return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
}

function pretty(v: unknown): string {
  if (typeof v === "string") {
    try {
      return JSON.stringify(JSON.parse(v), null, 2);
    } catch {
      return v;
    }
  }
  try {
    return JSON.stringify(v, null, 2);
  } catch {
    return String(v ?? "");
  }
}

function pickNum(o: Record<string, unknown>, ...keys: string[]): number | undefined {
  for (const k of keys) {
    const v = o[k];
    if (typeof v === "number" && Number.isFinite(v)) return v;
  }
  return undefined;
}

function pickBody(o: Record<string, unknown>): unknown {
  return o.body ?? o.redacted_body ?? o.redactedBody ?? o.data ?? o;
}

export function ApiPane({ pane }: { pane: PaneState }) {
  const devices = useApp((s) => s.inventory?.devices ?? []);
  const deviceId = pane.deviceId ?? pane.session?.device_id ?? undefined;
  const device = useMemo(() => devices.find((d) => d.id === deviceId), [devices, deviceId]);
  const [method, setMethod] = useState<(typeof METHODS)[number]>("GET");
  const [url, setUrl] = useState("");
  const [headers, setHeaders] = useState('{\n  "Accept": "application/json"\n}');
  const [body, setBody] = useState("{}");
  const [status, setStatus] = useState<number | null>(null);
  const [elapsed, setElapsed] = useState<number | null>(null);
  const [respHeaders, setRespHeaders] = useState("");
  const [respBody, setRespBody] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const base = device?.api_base_url?.trim();
    if (base) setUrl((u) => (u.trim() ? u : base));
  }, [device?.api_base_url]);

  async function send() {
    if (!url.trim()) {
      toast("error", "Enter a URL or path.");
      return;
    }
    if (!deviceId && !pane.session?.id) {
      toast("error", "Open an API session from the sidebar so requests stay host-pinned.");
      return;
    }
    setBusy(true);
    setStatus(null);
    setElapsed(null);
    setRespHeaders("");
    setRespBody("");
    try {
      let parsedHeaders: Record<string, string> = {};
      try {
        parsedHeaders = JSON.parse(headers || "{}") as Record<string, string>;
      } catch {
        throw new Error("Headers must be JSON, e.g. {\"Accept\":\"application/json\"}");
      }
      let parsedBody: unknown;
      if (method !== "GET" && body.trim()) {
        try {
          parsedBody = JSON.parse(body);
        } catch {
          throw new Error("Body must be JSON");
        }
      }
      const r = await rpc.call<unknown>(
        "api.request",
        {
          deviceId,
          device_id: deviceId,
          sessionId: pane.session?.id,
          session_id: pane.session?.id,
          method,
          url: url.trim(),
          headers: parsedHeaders,
          body: parsedBody,
        },
        60_000,
      );
      const rec = asRecord(r);
      if (rec) {
        const st = pickNum(rec, "status", "statusCode", "status_code");
        setStatus(st ?? null);
        setElapsed(pickNum(rec, "elapsedMs", "elapsed_ms", "elapsed") ?? null);
        const hdrs = rec.headers;
        setRespHeaders(hdrs ? pretty(hdrs) : "");
        setRespBody(pretty(pickBody(rec)));
      } else {
        setRespBody(pretty(r));
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setRespBody(message);
      toast("error", message);
    } finally {
      setBusy(false);
    }
  }

  const statusClass = status == null ? "" : status >= 200 && status < 400 ? "ok" : "bad";

  return (
    <div className="api-pane">
      {!deviceId && !pane.session?.id && (
        <div className="sftp-banner">API session not connected. Connect an API device so the daemon can pin the host and attach credentials.</div>
      )}
      <div className="api-bar">
        <select value={method} onChange={(e) => setMethod(e.target.value as (typeof METHODS)[number])}>
          {METHODS.map((m) => (
            <option key={m}>{m}</option>
          ))}
        </select>
        <input
          placeholder={device?.api_base_url || "https://controller/api/..."}
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) void send();
          }}
        />
        <button className="primary" disabled={busy} onClick={() => void send()}>
          {busy ? "Sending…" : "Send"}
        </button>
      </div>
      <div className={`api-status ${statusClass}`}>
        {status != null ? `HTTP ${status}` : "No response yet"}
        {elapsed != null ? ` · ${elapsed} ms` : ""}
        <span className="sftp-hint">Agent may only propose GET on this host (FortiManager JSON-RPC is human-only).</span>
      </div>
      <div className="api-split">
        <div className="api-col">
          <label>
            Headers JSON
            <textarea value={headers} onChange={(e) => setHeaders(e.target.value)} />
          </label>
          <label>
            Body JSON {method === "GET" ? "(ignored for GET)" : ""}
            <textarea value={body} onChange={(e) => setBody(e.target.value)} disabled={method === "GET"} />
          </label>
        </div>
        <div className="api-col">
          <label>
            Response headers
            <textarea readOnly value={respHeaders} />
          </label>
          <label>
            Response body
            <textarea readOnly value={respBody} className="api-body" />
          </label>
        </div>
      </div>
    </div>
  );
}
