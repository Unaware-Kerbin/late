import { useEffect, useRef, useState, type DragEvent, type MouseEvent, type ReactNode } from "react";
import { vllmDockerHint, vllmStartVisible } from "../lib/inferenceUi";
import { isProposalDismissed } from "../lib/approvals-ui";
import {
  sidecarHealth,
  sidecarGrantDir,
  sidecarModels,
  sidecarPending,
  sidecarProbe,
  stopChat,
  streamChat,
  type ChatTargetMeta,
  type SidecarModels,
} from "../lib/sidecar";
import {
  downloadInference,
  inferenceStatus,
  dockAgent,
  openStagePane,
  saveSettings,
  setChatModelsH,
  setState,
  startInference,
  stopInference,
  useApp,
  widenChat,
  type InferenceStatus,
  type LocalModel,
} from "../store";
import { stageOpenFromToolDetail } from "../lib/stageDrafts";
import { onShellDragEnd, onShellDragStart } from "../shellDnD";
import { isAgentStacked, MODELS_H } from "../shell";
import {
  backendLabel,
  coerceChatBackend,
  DEFAULT_MCP_HTTP_URL,
  inferenceHostLabel,
  isInferenceEngine,
  isLoopbackInferenceUrl,
  LOCAL_INFERENCE_URL,
  mcpPickMode,
  mcpLocationLabel,
  mcpStatusLine,
  newId,
  remoteInferenceUrl,
  usingRemoteInference,
  withInferenceServer,
  withMcpPick,
  type AppSettings,
  type ChatBackend,
  type ChatMsg,
  type InferenceEngineBackend,
} from "../types";

/** MCP Agent dropdown: orchestrator auto-route only. Other backends stay in the first menu. */
const MCP_MODEL = "orchestrator";

function targetOf(models: SidecarModels, backend: ChatBackend): ChatTargetMeta | undefined {
  if (backend === "ollama") return models.targets?.ollama;
  if (backend === "llamacpp") return models.targets?.llamacpp;
  if (backend === "local") return models.targets?.local;
  if (backend === "anthropic") return models.targets?.anthropic;
  if (backend === "gemini") return models.targets?.gemini;
  if (backend === "azure") return models.targets?.azure;
  if (backend === "mcp") return models.targets?.mcp;
  return undefined;
}

function whereServer(t?: ChatTargetMeta): string {
  if (!t?.base) return "";
  if (t.egress === "private") return `your network · ${t.base}`;
  if (t.egress === "cloud") return `public internet · ${t.base}`;
  return `this computer · ${t.base}`;
}

/** Never show orchestrator thread JSON or Late SYSTEM wrap after “Agent stopped:”. */
function agentStopLine(message: string): string {
  const t = message.trim();
  if (!t) return "MCP chat ended.";
  if (/^SYSTEM:/m.test(t) && /UNTRUSTED DEVICE OUTPUT/i.test(t)) {
    return "MCP debate finished with skipped speakers. Successful replies are above.";
  }
  if ((t.startsWith("{") || t.startsWith("[")) && /"messages"\s*:/.test(t)) {
    return "MCP debate finished with skipped speakers. Successful replies are above.";
  }
  const first = t.split(/\n/)[0]?.trim() ?? t;
  return first.length > 200 ? `${first.slice(0, 180)}…` : first;
}

function fmtSize(n: number) {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)} GB`;
  if (n >= 1_000_000) return `${Math.round(n / 1_000_000)} MB`;
  if (n <= 0) return "";
  return `${n} B`;
}

function idleGpu(): ReturnType<typeof fromStatus> {
  return {
    running: false,
    starting: false,
    downloading: false,
    models: [],
    localModels: [],
    gpu: undefined,
    detail: "checking…",
    allowIntelCompose: false,
    lateOwned: false,
    gpuLaunch: undefined,
    dockerAvailable: false,
  };
}

function fromStatus(s: InferenceStatus) {
  return {
    running: s.running,
    starting: Boolean(s.starting),
    downloading: Boolean(s.downloading),
    models: s.models ?? [],
    localModels: s.localModels ?? [],
    gpu: s.gpu,
    detail: s.detail,
    allowIntelCompose: Boolean(s.allowIntelCompose),
    lateOwned: Boolean(s.lateOwned),
    gpuLaunch: s.gpuLaunch,
    dockerAvailable: Boolean(s.dockerAvailable),
  };
}

function serveStorageKey(backend: ChatBackend) {
  if (backend === "llamacpp") return "late.ggufServe";
  if (backend === "ollama") return "late.ollamaServe";
  return "late.serveModel";
}

function defaultHubId(backend: ChatBackend) {
  if (backend === "llamacpp") return "Qwen/Qwen3-8B-GGUF";
  if (backend === "ollama") return "qwen3:8b";
  return "Qwen/Qwen3-8B";
}

function catalogFitLabel(m: { recommended?: boolean; newest?: boolean; tp?: number; note?: string; complete?: boolean }) {
  const fit = m.recommended
    ? m.tp && m.tp > 1
      ? "needs TP"
      : "fits"
    : (m.note || "").toLowerCase().includes("needs all")
      ? "needs TP"
      : m.complete
        ? "cached"
        : "too big";
  const gen = m.newest ? "newest" : "previous";
  return `${fit} · ${gen}`;
}

function inferenceEngine(backend: ChatBackend): "vllm" | "llamacpp" | "ollama" | null {
  if (backend === "llamacpp") return "llamacpp";
  if (backend === "ollama") return "ollama";
  if (backend === "local") return "vllm";
  return null;
}

function probeKind(backend: InferenceEngineBackend): "vllm" | "llamacpp" | "ollama" {
  if (backend === "llamacpp") return "llamacpp";
  if (backend === "ollama") return "ollama";
  return "vllm";
}

function InferenceServerPick(props: {
  backend: InferenceEngineBackend;
  settings: AppSettings | null;
  onApplied: () => void;
  resetChat: (label: string) => void;
}) {
  const { backend, settings } = props;
  const [open, setOpen] = useState(false);
  const [dialog, setDialog] = useState(false);
  const [url, setUrl] = useState("");
  const [check, setCheck] = useState<{ busy: boolean; ok?: boolean; message: string } | null>(null);
  const wrap = useRef<HTMLDivElement>(null);
  const remote = remoteInferenceUrl(settings, backend);
  const usingRemote = usingRemoteInference(settings, backend);
  const trigger = usingRemote && remote ? inferenceHostLabel(remote) : "Local";

  useEffect(() => {
    if (!open) return;
    function onDoc(e: globalThis.MouseEvent) {
      if (wrap.current && !wrap.current.contains(e.target as Node)) setOpen(false);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  async function apply(next: { base: string; remote: string }, quiet: boolean, chatLabel: string) {
    if (!settings) return;
    const ok = await saveSettings(withInferenceServer(settings, backend, next), { quiet });
    if (!ok) return;
    props.resetChat(chatLabel);
    props.onApplied();
  }

  function openAdd() {
    setOpen(false);
    setUrl(remote || "");
    setCheck(null);
    setDialog(true);
  }

  async function pickLocal() {
    setOpen(false);
    if (!usingRemote) return;
    await apply({ base: LOCAL_INFERENCE_URL[backend], remote }, true, `${backendLabel(backend)} · Local`);
  }

  async function pickRemote() {
    setOpen(false);
    if (!remote || usingRemote) return;
    await apply({ base: remote, remote }, true, `${backendLabel(backend)} · ${inferenceHostLabel(remote)}`);
  }

  async function saveRemote() {
    const next = url.trim();
    if (!next) {
      setCheck({ busy: false, ok: false, message: "Enter a URL." });
      return;
    }
    let parsed: URL;
    try {
      parsed = new URL(next);
    } catch {
      setCheck({ busy: false, ok: false, message: "Need an http(s) URL, for example http://10.0.0.12:8000/v1." });
      return;
    }
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      setCheck({ busy: false, ok: false, message: "URL must be http or https." });
      return;
    }
    if (isLoopbackInferenceUrl(next)) {
      setCheck({ busy: false, ok: false, message: "That is this computer. Pick Local in the menu instead." });
      return;
    }
    await apply(
      { base: next, remote: next },
      false,
      `${backendLabel(backend)} · ${inferenceHostLabel(next)}`,
    );
    setDialog(false);
  }

  async function removeRemote() {
    await apply(
      { base: LOCAL_INFERENCE_URL[backend], remote: "" },
      false,
      `${backendLabel(backend)} · Local`,
    );
    setDialog(false);
  }

  async function checkUrl() {
    setCheck({ busy: true, message: "Checking…" });
    try {
      const r = await sidecarProbe(probeKind(backend), url.trim());
      setCheck({ busy: false, ok: r.ok, message: r.message });
    } catch (err) {
      setCheck({ busy: false, ok: false, message: err instanceof Error ? err.message : String(err) });
    }
  }

  return (
    <div className="server-pick" ref={wrap}>
      <button
        type="button"
        className="server-pick-btn"
        title="This computer or a saved helper on your network"
        aria-expanded={open}
        aria-haspopup="listbox"
        onClick={() => setOpen((v) => !v)}
      >
        {trigger}
      </button>
      {open ? (
        <div className="server-menu" role="listbox">
          <button type="button" className={!usingRemote ? "on" : undefined} onClick={() => void pickLocal()}>
            Local
          </button>
          {remote ? (
            <button type="button" className={usingRemote ? "on" : undefined} onClick={() => void pickRemote()}>
              {inferenceHostLabel(remote)}
            </button>
          ) : null}
          <button type="button" className="server-add" onClick={openAdd}>
            Add server
          </button>
        </div>
      ) : null}
      {dialog ? (
        <div
          className="modal-root"
          onMouseDown={() => setDialog(false)}
        >
          <div
            className="modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="late-add-server-title"
            onMouseDown={(e) => e.stopPropagation()}
          >
            <h2 id="late-add-server-title">{remote ? `Edit ${backendLabel(backend)} server` : `Add ${backendLabel(backend)} server`}</h2>
            <p className="hint">
              A server you already run on your network. Late will not start it. OpenAI-compatible
              <code> /v1</code> — same as Settings. Cloud AI stays off for a private IP.
            </p>
            <div className="url-check">
              <label>
                Base URL
                <input
                  value={url}
                  autoFocus
                  placeholder={LOCAL_INFERENCE_URL[backend].replace("127.0.0.1", "10.0.0.12")}
                  onChange={(e) => setUrl(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void saveRemote();
                    if (e.key === "Escape") setDialog(false);
                  }}
                />
              </label>
              <button type="button" className="ghost" disabled={check?.busy || !url.trim()} onClick={() => void checkUrl()}>
                {check?.busy ? "Checking…" : "Check"}
              </button>
            </div>
            {check?.message ? <p className={check.ok ? "hint" : "hint warn"}>{check.message}</p> : null}
            <div className="actions">
              {remote ? (
                <button type="button" className="ghost" onClick={() => void removeRemote()}>
                  Remove
                </button>
              ) : null}
              <button type="button" className="ghost" onClick={() => setDialog(false)}>
                Cancel
              </button>
              <button type="button" className="primary" onClick={() => void saveRemote()}>
                Save
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}

const MCP_HTTP_STASH = "late.mcpHttpUrl";

function stashMcpHttpUrl(url: string) {
  const t = url.trim();
  if (!t) return;
  try {
    localStorage.setItem(MCP_HTTP_STASH, t);
  } catch {
    /* ignore */
  }
}

function stashedMcpHttpUrl(): string {
  try {
    return localStorage.getItem(MCP_HTTP_STASH)?.trim() ?? "";
  } catch {
    return "";
  }
}

function savedMcpHttpUrl(settings: AppSettings | null): string {
  return settings?.mcp_url?.trim() || stashedMcpHttpUrl();
}

function McpServerPick(props: {
  settings: AppSettings | null;
  onApplied: () => void;
  resetChat: (label: string) => void;
  onHint: (text: string) => void;
}) {
  const { settings } = props;
  const [open, setOpen] = useState(false);
  const [dialog, setDialog] = useState(false);
  const [url, setUrl] = useState("");
  const [check, setCheck] = useState<{ busy: boolean; ok?: boolean; message: string } | null>(null);
  const wrap = useRef<HTMLDivElement>(null);
  const mode = mcpPickMode(settings);
  const saved = savedMcpHttpUrl(settings);
  const trigger = mcpLocationLabel(settings);

  useEffect(() => {
    if (!open) return;
    function onDoc(e: globalThis.MouseEvent) {
      if (wrap.current && !wrap.current.contains(e.target as Node)) setOpen(false);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  async function apply(next: { enabled: boolean; url: string }, quiet: boolean, chatLabel: string) {
    if (!settings) return;
    if (next.url.trim()) stashMcpHttpUrl(next.url);
    const ok = await saveSettings(withMcpPick(settings, next), { quiet });
    if (!ok) return;
    props.resetChat(chatLabel);
    props.onApplied();
  }

  function openHttp() {
    setOpen(false);
    setUrl(saved || DEFAULT_MCP_HTTP_URL);
    setCheck(null);
    setDialog(true);
  }

  async function pickThisComputer() {
    setOpen(false);
    if (mode === "stdio") return;
    const prev = settings?.mcp_url?.trim() ?? "";
    if (prev) stashMcpHttpUrl(prev);
    await apply({ enabled: true, url: "" }, true, "MCP · this computer");
    if (!settings?.mcp_cwd?.trim()) {
      props.onHint("Set the MCP folder in Settings to use tools on your computer.");
    }
  }

  async function pickSavedHttp() {
    setOpen(false);
    if (!saved || (mode === "http" && settings?.mcp_url?.trim() === saved)) return;
    await apply({ enabled: true, url: saved }, true, `MCP · ${inferenceHostLabel(saved)}`);
  }

  async function saveHttp() {
    const next = url.trim();
    if (!next) {
      setCheck({ busy: false, ok: false, message: "Enter a URL." });
      return;
    }
    let parsed: URL;
    try {
      parsed = new URL(next);
    } catch {
      setCheck({
        busy: false,
        ok: false,
        message: "Need an http(s) URL, for example the /mcp address printed by the GUI or npm run mcp:http.",
      });
      return;
    }
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      setCheck({ busy: false, ok: false, message: "URL must be http or https." });
      return;
    }
    stashMcpHttpUrl(next);
    await apply({ enabled: true, url: next }, false, `MCP · ${inferenceHostLabel(next)}`);
    setDialog(false);
  }

  async function removeHttp() {
    try {
      localStorage.removeItem(MCP_HTTP_STASH);
    } catch {
      /* ignore */
    }
    const cwd = settings?.mcp_cwd?.trim() ?? "";
    if (cwd) {
      await apply({ enabled: true, url: "" }, false, "MCP · this computer");
    } else {
      await apply({ enabled: true, url: "" }, false, "MCP");
    }
    setDialog(false);
  }

  async function checkUrl() {
    setCheck({ busy: true, message: "Checking…" });
    try {
      const r = await sidecarProbe("mcp", url.trim());
      setCheck({ busy: false, ok: r.ok, message: r.message });
    } catch (err) {
      setCheck({ busy: false, ok: false, message: err instanceof Error ? err.message : String(err) });
    }
  }

  return (
    <div className="server-pick" ref={wrap}>
      <button
        type="button"
        className="server-pick-btn"
        title="This computer (folder) or an HTTP address you already started"
        aria-label="MCP location"
        aria-expanded={open}
        aria-haspopup="listbox"
        onClick={() => setOpen((v) => !v)}
      >
        {trigger}
      </button>
      {open ? (
        <div className="server-menu" role="listbox">
          <button
            type="button"
            className={mode === "stdio" ? "on" : undefined}
            onClick={() => void pickThisComputer()}
          >
            This computer
          </button>
          {saved ? (
            <button
              type="button"
              className={mode === "http" && settings?.mcp_url?.trim() === saved ? "on" : undefined}
              onClick={() => void pickSavedHttp()}
            >
              {inferenceHostLabel(saved)}
            </button>
          ) : null}
          <button type="button" className="server-add" onClick={openHttp}>
            HTTP address
          </button>
        </div>
      ) : null}
      {dialog ? (
        <div className="modal-root" onMouseDown={() => setDialog(false)}>
          <div
            className="modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="late-mcp-http-title"
            onMouseDown={(e) => e.stopPropagation()}
          >
            <h2 id="late-mcp-http-title">{saved ? "Edit MCP address" : "MCP address"}</h2>
            <p className="hint">
              A program you already started. Late will not start it. Streamable HTTP — same as Settings.
              Use the <code>/mcp</code> URL printed when you started the GUI or <code>npm run mcp:http</code>
              (default example <code>{DEFAULT_MCP_HTTP_URL}</code> if you did not set a port). Late Settings is the source of truth. Cloud AI stays off for a private IP.
            </p>
            <div className="url-check">
              <label>
                MCP address
                <input
                  value={url}
                  autoFocus
                  placeholder={DEFAULT_MCP_HTTP_URL}
                  onChange={(e) => setUrl(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void saveHttp();
                    if (e.key === "Escape") setDialog(false);
                  }}
                />
              </label>
              <button type="button" className="ghost" disabled={check?.busy || !url.trim()} onClick={() => void checkUrl()}>
                {check?.busy ? "Checking…" : "Check"}
              </button>
            </div>
            {check?.message ? <p className={check.ok ? "hint" : "hint warn"}>{check.message}</p> : null}
            <div className="actions">
              {saved ? (
                <button type="button" className="ghost" onClick={() => void removeHttp()}>
                  Remove
                </button>
              ) : null}
              <button type="button" className="ghost" onClick={() => setDialog(false)}>
                Cancel
              </button>
              <button type="button" className="primary" onClick={() => void saveHttp()}>
                Save
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}

export function ChatPane() {
  const approval = useApp((s) => s.approval);
  const agentSeed = useApp((s) => s.agentSeed);
  const sessions = useApp((s) => s.sessions);
  const settings = useApp((s) => s.settings);
  const layout = useApp((s) => s.layout);
  const stacked = isAgentStacked(layout);
  const attached = (sessions ?? []).filter((s) => String(s.kind).toLowerCase() !== "sftp");
  const [messages, setMessages] = useState<ChatMsg[]>([]);
  const [input, setInput] = useState("");
  const [backend, setBackend] = useState<ChatBackend>("local");
  const [model, setModel] = useState("");
  const [models, setModels] = useState<SidecarModels>({
    local: [],
    cursor: [],
    ollama: [],
    llamacpp: [],
    anthropic: [],
    gemini: [],
    azure: [],
    backends: ["local", "llamacpp", "ollama", "anthropic", "gemini", "azure", "cursor", "mcp"],
  });
  const [busy, setBusy] = useState(false);
  const [dropOver, setDropOver] = useState(false);
  const [thinking, setThinking] = useState("");
  const [thinkMs, setThinkMs] = useState(0);
  const [status, setStatus] = useState("sidecar idle");
  const [gpu, setGpu] = useState<ReturnType<typeof fromStatus>>({
    running: false,
    starting: false,
    downloading: false,
    models: [] as string[],
    localModels: [] as LocalModel[],
    gpu: undefined as InferenceStatus["gpu"],
    detail: "checking…",
    allowIntelCompose: false,
    lateOwned: false,
    gpuLaunch: undefined,
    dockerAvailable: false,
  });
  const [gpuBusy, setGpuBusy] = useState(false);
  const [serveModel, setServeModel] = useState(() => {
    try {
      return localStorage.getItem("late.serveModel") || "";
    } catch {
      return "";
    }
  });
  const [downloadId, setDownloadId] = useState("Qwen/Qwen3-8B");
  const conv = useRef(newId());
  const abort = useRef<AbortController>();
  const busyRef = useRef(false);
  const queuedGrantRef = useRef<string | null>(null);
  const backendRef = useRef(backend);
  backendRef.current = backend;
  const settingsApplied = useRef(false);

  async function refreshModels() {
    const m = await sidecarModels();
    setModels(m);
    setModel((cur) => {
      const b = backendRef.current;
      if (b === "mcp") return MCP_MODEL;
      if (cur) return cur;
      if (b === "ollama") return m.ollama[0] || "";
      if (b === "llamacpp") return m.llamacpp[0] || "";
      if (b === "anthropic") return m.anthropic?.[0] || "claude-sonnet-4-5";
      if (b === "gemini") return m.gemini?.[0] || "gemini-2.5-flash";
      if (b === "azure") return m.azure?.[0] || "";
      if (b === "cursor") return m.cursor[0] || "composer-2.5";
      return m.local[0] || "local";
    });
  }

  useEffect(() => {
    void sidecarHealth().then((ok) => {
      setState({ sidecarOk: ok });
      setStatus(ok ? "sidecar online" : "sidecar offline");
    });
    void refreshModels().catch((e: unknown) => setStatus(e instanceof Error ? e.message : String(e)));
  }, []);

  useEffect(() => {
    if (!settings) return;
    void refreshModels().catch((e: unknown) => setStatus(e instanceof Error ? e.message : String(e)));
  }, [settings?.vllm_base_url, settings?.llama_cpp_base_url, settings?.ollama_base_url, settings?.mcp_enabled, settings?.mcp_cwd, settings?.mcp_url]);

  useEffect(() => {
    if (backend !== "mcp") return;
    const tick = () => void refreshModels().catch(() => undefined);
    tick();
    const id = window.setInterval(tick, 5000);
    return () => window.clearInterval(id);
  }, [backend]);

  useEffect(() => {
    const engine = inferenceEngine(backend);
    if (!engine) return;
    setGpu(idleGpu());
    setDownloadId(defaultHubId(backend));
    try {
      const saved = localStorage.getItem(serveStorageKey(backend));
      if (saved) setServeModel(saved);
    } catch {
      /* ignore */
    }
    const tick = () => {
      void inferenceStatus(engine)
        .then((s) => {
          setGpu(fromStatus(s));
          setServeModel((cur) => {
            const ids = (s.localModels ?? []).map((m) => m.id);
            if (cur && ids.includes(cur)) return cur;
            const rec = (s.localModels ?? []).find((m) => m.recommended && m.newest);
            const complete = (s.localModels ?? []).find((m) => m.complete && m.recommended && m.newest);
            const next = complete?.id || rec?.id || s.localModels?.[0]?.id || cur || defaultHubId(backend);
            try {
              if (next) localStorage.setItem(serveStorageKey(backend), next);
            } catch {
              /* ignore */
            }
            return next;
          });
          if (s.running && s.models?.length) void refreshModels().catch(() => undefined);
        })
        .catch(() =>
          setGpu((g) => ({
            ...g,
            detail: g.starting || g.running
              ? `${g.detail} (status poll delayed — sessions should stay live)`
              : "daemon inference API unavailable",
          })),
        );
      void refreshModels().catch(() => undefined);
    };
    tick();
    const id = window.setInterval(tick, 5000);
    return () => window.clearInterval(id);
  }, [backend]);

  useEffect(() => {
    if (!busy) {
      setThinkMs(0);
      return;
    }
    const t0 = Date.now();
    const id = window.setInterval(() => setThinkMs(Date.now() - t0), 250);
    return () => window.clearInterval(id);
  }, [busy]);

  function resetChat(opts?: { backend?: ChatBackend; model?: string; label?: string }) {
    abort.current?.abort();
    void stopChat(conv.current);
    conv.current = newId();
    queuedGrantRef.current = null;
    busyRef.current = false;
    setMessages([]);
    setInput("");
    setBusy(false);
    setThinking("");
    setState({ approval: null });
    const b = opts?.backend ?? backend;
    if (opts?.backend) {
      if (b === "ollama") setModel(opts.model || models.ollama[0] || "");
      else if (b === "llamacpp") setModel(opts.model || models.llamacpp[0] || "");
      else if (b === "local") setModel(opts.model || models.local[0] || "local");
      else if (b === "anthropic") setModel(opts.model || models.anthropic?.[0] || "claude-sonnet-4-5");
      else if (b === "gemini") setModel(opts.model || models.gemini?.[0] || "gemini-2.5-flash");
      else if (b === "azure") setModel(opts.model || models.azure?.[0] || "");
      else if (b === "mcp") setModel(MCP_MODEL);
      else setModel(opts.model || models.cursor[0] || "composer-2.5");
    } else if (opts?.model) {
      setModel(opts.model);
    }
    const name = opts?.label || backendLabel(b);
    setStatus(`new chat · ${name}`);
  }

  useEffect(() => {
    if (settingsApplied.current || !settings) return;
    settingsApplied.current = true;
    const next = coerceChatBackend(settings.default_backend);
    const cloudOk = Boolean(settings.cloud_chat_enabled);
    const backendNext = next === "cursor" && !cloudOk ? "local" : next;
    setBackend(backendNext);
    if (backendNext === "ollama") setModel(settings.ollama_model || models.ollama[0] || "");
    else if (backendNext === "llamacpp") setModel(settings.llama_cpp_model || models.llamacpp[0] || "");
    else if (backendNext === "anthropic") setModel(settings.anthropic_model || models.anthropic?.[0] || "claude-sonnet-4-5");
    else if (backendNext === "gemini") setModel(settings.gemini_model || models.gemini?.[0] || "gemini-2.5-flash");
    else if (backendNext === "azure") setModel(settings.azure_deployment || models.azure?.[0] || "");
    else if (backendNext === "cursor") setModel(settings.cursor_model || models.cursor[0] || "composer-2.5");
    else if (backendNext === "mcp") setModel(MCP_MODEL);
    else setModel(settings.vllm_model || models.local[0] || "local");
  }, [settings, models.cursor, models.local, models.ollama, models.llamacpp, models.anthropic, models.gemini, models.azure]);

  useEffect(() => {
    let stop = false;
    let emptyTicks = 0;
    const tick = async () => {
      if (stop) return;
      const pending = await sidecarPending();
      const first = pending.find(
        (p) => p.proposalId && p.detail && !isProposalDismissed(p.proposalId),
      );
      if (first) {
        emptyTicks = 0;
        setState((s) => {
          if (isProposalDismissed(first.proposalId)) return {};
          if (s.approval?.proposalId === first.proposalId) return {};
          return { approval: first };
        });
        return;
      }
      if (!busy) {
        emptyTicks += 1;
        if (emptyTicks >= 8) {
          stop = true;
          window.clearInterval(id);
        }
      }
    };
    const id = window.setInterval(() => void tick(), 700);
    void tick();
    return () => {
      stop = true;
      window.clearInterval(id);
    };
  }, [busy]);

  useEffect(() => {
    if (!agentSeed?.text) return;
    const seed = agentSeed;
    setState({ agentSeed: null });
    if (seed.autoSend) void sendText(seed.text);
    else setInput(seed.text);
  }, [agentSeed]);

  async function send() {
    await sendText(input);
  }

  async function sendText(raw: string) {
    const text = raw.trim();
    if (!text || busyRef.current) return;
    if (backend === "cursor" && !settings?.cloud_chat_enabled) {
      setStatus("Cloud AI is off. Turn it on in Settings to use Cursor.");
      setState({ settingsOpen: true });
      return;
    }
    if (
      (backend === "local" || backend === "llamacpp" || backend === "ollama") &&
      (gpu.starting || gpu.downloading) &&
      !gpu.running &&
      gpu.models.length === 0
    ) {
      setStatus(
        "wait until GPU status is running before sending — first load can take several minutes (do not click Stop)",
      );
      return;
    }
    const next: ChatMsg[] = [...messages, { id: newId(), role: "user", content: text }];
    setMessages(next);
    setInput("");
    busyRef.current = true;
    setBusy(true);
    setThinking("Thinking");
    setState({ approval: null });
    abort.current = new AbortController();
    let assistant = "";
    let speakersSeen = false;
    const id = newId();
    try {
      await streamChat({
        messages: next,
        backend,
        model: model || undefined,
        conversationId: conv.current,
        signal: abort.current.signal,
        onEvent: (e) => {
          if (e.type === "speaker") {
            speakersSeen = true;
            setThinking("Writing");
            setMessages((cur) => [
              ...cur,
              {
                id: newId(),
                role: "assistant",
                content: e.content,
                speaker: e.speaker,
                label: e.label,
                nickname: e.nickname,
                logoDataUrl: e.logoDataUrl,
                skipped: e.skipped === true,
              },
            ]);
          } else if (e.type === "delta") {
            if (speakersSeen) return;
            assistant += e.text;
            setThinking("Writing");
            setMessages([...next, { id, role: "assistant", content: assistant }]);
          } else if (e.type === "tool") {
            const name = e.name || "tool";
            if (e.status === "start") setThinking(`Using ${name.replace(/_/g, " ")}`);
            else if (e.status === "ok") {
              setThinking("Thinking about the result");
              if (name === "propose_staged_artifact") {
                openStagePane(stageOpenFromToolDetail(e.detail));
              }
            }
            else if (e.status === "denied") {
              const reason =
                typeof e.detail === "string" ? e.detail : JSON.stringify(e.detail ?? "permit list denied");
              setStatus(`permit denied: ${reason}`);
              setMessages((cur) => [
                ...cur,
                { id: newId(), role: "system", content: `Permit list denied: ${reason}` },
              ]);
            }
          } else if (e.type === "approval") {
            if (e.pending?.proposalId && isProposalDismissed(e.pending.proposalId)) {
              /* already Cancelled / Approved */
            } else {
              setThinking("Waiting for you to Approve");
              setState({ approval: e.pending });
            }
          } else if (e.type === "ask") {
            if (isProposalDismissed(e.proposalId)) {
              /* already Cancelled / Approved */
            } else {
              setThinking("Waiting for your answer");
              setState({
                approval: {
                  proposalId: e.proposalId,
                  kind: "ask",
                  title: "Agent question",
                  detail: { question: e.question },
                },
              });
            }
          } else if (e.type === "error") {
            setStatus(e.message);
            setThinking("");
            const line = agentStopLine(e.message);
            const prefix =
              /MCP HTTP\s+[1-5]\d\d|MCP unreachable|MCP is not reachable|MCP is off|no chat_send/i.test(
                e.message,
              )
                ? "Agent error"
                : "Agent stopped";
            setMessages((cur) => [
              ...cur,
              { id: newId(), role: "system", content: `${prefix}: ${line}` },
            ]);
          } else if (e.type === "round") {
            setThinking(`Thinking · round ${e.n}/${e.max}`);
            setStatus(`round ${e.n}/${e.max}`);
          } else if (e.type === "heartbeat") {
            setThinking(e.message || "Still thinking");
            setStatus(e.message || "still thinking");
          } else if (e.type === "done") {
            setThinking("");
            if (!assistant && !speakersSeen) setMessages([...next, { id, role: "assistant", content: e.message }]);
          }
        },
      });
    } catch (err) {
      setStatus(err instanceof Error ? err.message : String(err));
      setThinking("");
    } finally {
      const queued = queuedGrantRef.current;
      queuedGrantRef.current = null;
      if (queued) {
        await runFolderGrant(queued);
      } else {
        busyRef.current = false;
        setBusy(false);
        setThinking("");
      }
    }
  }

  function parentOfPath(p: string): string {
    const trimmed = p.replace(/[\\/]+$/, "");
    const idx = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
    return idx > 0 ? trimmed.slice(0, idx) : trimmed;
  }

  async function pathFromDrop(dt: DataTransfer): Promise<string> {
    try {
      const file = dt.files?.[0];
      if (!file) return "";
      const raw = window.lateRuntime?.pathForFile?.(file)?.trim() || "";
      if (!raw) return "";
      let isDir: boolean | undefined;
      try {
        isDir = await window.lateRuntime?.isDirectory?.(raw);
      } catch {
        isDir = undefined;
      }
      return isDir === false ? parentOfPath(raw) : raw;
    } catch {
      return "";
    }
  }

  async function runFolderGrant(folder: string) {
    busyRef.current = true;
    setBusy(true);
    setThinking("Waiting for you to Approve the folder grant");
    setStatus("Approve to grant this folder on the MCP host");
    try {
      const result = await sidecarGrantDir(folder);
      if (result.denied) setStatus("Folder grant cancelled");
      else if (result.ok) {
        setStatus(
          `Granted ${result.cwd ?? folder}. Later chat uses it as cwd. The path must already exist on the MCP host.`,
        );
      } else {
        setStatus(result.error || result.note || "Grant failed — path must exist on the MCP host");
      }
    } catch (err) {
      setStatus(err instanceof Error ? err.message : String(err));
    } finally {
      busyRef.current = false;
      setBusy(false);
      setThinking("");
    }
  }

  async function onFolderDrop(event: DragEvent<HTMLElement>) {
    if (!event.dataTransfer?.types?.includes("Files")) return;
    event.preventDefault();
    event.stopPropagation();
    setDropOver(false);
    let folder = "";
    try {
      folder = await pathFromDrop(event.dataTransfer);
    } catch {
      folder = "";
    }
    if (!folder) {
      setStatus(
        "Could not read that folder path. Type mcp_cwd in Settings, or drop a folder in Late (Electron), not a browser tab.",
      );
      return;
    }
    if (backend !== "mcp") {
      setStatus(`Switch Agent to MCP to grant ${folder}. Typed mcp_cwd in Settings is the fallback.`);
      return;
    }
    if (busyRef.current) {
      queuedGrantRef.current = folder;
      setStatus(`Will grant ${folder} after this turn. Path must exist on the MCP host.`);
      return;
    }
    await runFolderGrant(folder);
  }

  function pickModel(id: string) {
    setServeModel(id);
    setDownloadId(id);
    try {
      localStorage.setItem(serveStorageKey(backend), id);
    } catch {
      /* ignore */
    }
    if (backend === "ollama" || backend === "llamacpp") {
      setModel(id);
    }
  }

  async function fetchModel(id: string, engine: "vllm" | "llamacpp" | "ollama" = "vllm") {
    pickModel(id);
    setGpuBusy(true);
    try {
      setGpu(fromStatus(await downloadInference(id, engine)));
    } catch (e: unknown) {
      setGpu((g) => ({ ...g, detail: e instanceof Error ? e.message : String(e) }));
    } finally {
      setGpuBusy(false);
    }
  }

  async function setUseAllGpus(next: boolean) {
    if (!settings) return;
    const ok = await saveSettings({ ...settings, use_all_gpus: next }, { quiet: true });
    if (!ok) return;
    const engine = inferenceEngine(backend);
    if (!engine) return;
    try {
      setGpu(fromStatus(await inferenceStatus(engine)));
    } catch {
      /* status refresh is best-effort */
    }
  }

  const intelCompose = gpu.allowIntelCompose;
  const vllmEgress = targetOf(models, "local")?.egress;
  const vllmOffComputer = vllmEgress === "private" || vllmEgress === "cloud";
  const showVllmStart = vllmStartVisible({
    allowIntelCompose: intelCompose,
    dockerAvailable: gpu.dockerAvailable,
    networkServer: vllmOffComputer,
  });
  const dockerHint = vllmDockerHint({
    dockerAvailable: gpu.dockerAvailable,
    allowIntelCompose: intelCompose,
    networkServer: vllmOffComputer,
  });

  return (
    <aside
      className={`chat${dropOver ? " drop-over" : ""}`}
      onDragOver={(e) => {
        if (!e.dataTransfer?.types?.includes("Files")) return;
        e.preventDefault();
        setDropOver(true);
      }}
      onDragLeave={(e) => {
        if (e.currentTarget.contains(e.relatedTarget as Node)) return;
        setDropOver(false);
      }}
      onDrop={(e) => void onFolderDrop(e)}
    >
      <div className="chat-head">
        <span
          className="shell-drag"
          draggable
          title="Drag to move the agent pane"
          onDragStart={(e) => onShellDragStart(e, "chat")}
          onDragEnd={onShellDragEnd}
        >
          Agent
        </span>
        <span className="meta">
          {status}{backend === "mcp" ? ` · ${mcpStatusLine(models.mcp)}` : ""}
        </span>
        <span className="chat-head-actions">
          {stacked ? (
            <button type="button" className="ghost" title="Dock agent to the side" onClick={() => dockAgent("row")}>
              Side
            </button>
          ) : (
            <>
              <button type="button" className="ghost" title="Make the agent column wider" onClick={() => widenChat()}>
                Wide
              </button>
              <button type="button" className="ghost" title="Dock agent across the top" onClick={() => dockAgent("top")}>
                Top
              </button>
            </>
          )}
        </span>
      </div>
      <div className="chat-toolbar row">
        <select
          value={backend}
          onChange={(e) => {
            const next = coerceChatBackend(e.target.value);
            if (next === "cursor" && !settings?.cloud_chat_enabled) {
              setStatus("Turn on Cloud AI in Settings to use Cursor.");
              setState({ settingsOpen: true });
              return;
            }
            if (next === "mcp" && settings && !settings.mcp_enabled) {
              const url = settings.mcp_url?.trim() || stashedMcpHttpUrl() || "";
              void saveSettings(withMcpPick(settings, { enabled: true, url }), { quiet: true }).then((ok) => {
                if (!ok) return;
                setBackend("mcp");
                resetChat({ backend: "mcp", label: "MCP" });
                void refreshModels().catch((err: unknown) =>
                  setStatus(err instanceof Error ? err.message : String(err)),
                );
              });
              return;
            }
            setBackend(next);
            resetChat({ backend: next, label: backendLabel(next) });
          }}
        >
          <option value="local">vLLM</option>
          <option value="llamacpp">llama.cpp</option>
          <option value="ollama">Ollama</option>
          <option value="mcp">MCP</option>
          <option value="anthropic">Anthropic Claude</option>
          <option value="gemini">Gemini</option>
          <option value="azure">Azure</option>
          <option value="cursor" disabled={!settings?.cloud_chat_enabled}>
            Cursor SDK{settings?.cloud_chat_enabled ? "" : " — Cloud AI is off"}
          </option>
        </select>
        {isInferenceEngine(backend) ? (
          <InferenceServerPick
            backend={backend}
            settings={settings}
            onApplied={() => {
              void refreshModels().catch((e: unknown) =>
                setStatus(e instanceof Error ? e.message : String(e)),
              );
            }}
            resetChat={(label) => resetChat({ label })}
          />
        ) : null}
        <select
          title="Chat API model id after the server is up"
          value={model}
          onChange={(e) => {
            const next = e.target.value;
            if (next !== model) resetChat({ model: next, label: next });
          }}
        >
          {(backend === "ollama"
            ? (models.ollama.length ? models.ollama : ["(no Ollama models)"])
            : backend === "llamacpp"
              ? (models.llamacpp.length ? models.llamacpp : ["(no llama.cpp models)"])
              : backend === "anthropic"
                ? (models.anthropic?.length ? models.anthropic : ["claude-sonnet-4-5"])
                : backend === "gemini"
                  ? (models.gemini?.length ? models.gemini : ["gemini-2.5-flash"])
                  : backend === "azure"
                    ? (models.azure?.length ? models.azure : ["(set Azure deployment in Settings)"])
                    : backend === "local"
                      ? (models.local.length ? models.local : ["local"])
                      : backend === "mcp"
                        ? [MCP_MODEL]
                      : (models.cursor.length ? models.cursor : ["composer-2.5", "auto"])
          ).map((m) => (
            <option key={m} value={m} disabled={m.startsWith("(")}>{m}</option>
          ))}
        </select>
        {backend === "mcp" ? (
          <McpServerPick
            settings={settings}
            onApplied={() => {
              void refreshModels().catch((e: unknown) =>
                setStatus(e instanceof Error ? e.message : String(e)),
              );
            }}
            resetChat={(label) => resetChat({ label })}
            onHint={(text) => setStatus(text)}
          />
        ) : null}
      </div>
      {backend !== "cursor" && whereServer(targetOf(models, backend)) ? (
        <p className="meta" style={{ padding: "0 8px 6px", margin: 0 }}>
          Helper: {whereServer(targetOf(models, backend))}
        </p>
      ) : null}
      {backend === "local" && (
      <ModelsDock
        label="vLLM"
        status={gpu.downloading ? "downloading" : gpu.starting ? "starting" : gpu.running ? "running" : "stopped"}
      >
      <div className="chat-models-body">
        <p className="hint" style={{ margin: "0 0 8px" }}>
          {targetOf(models, "local")?.egress && targetOf(models, "local")?.egress !== "loopback"
            ? "Chat uses the saved vLLM server (another machine). Start/Download on this pane only manage a server on this computer — they stay hidden while the URL is not loopback. Pick Local to use this computer."
            : dockerHint
            ? dockerHint
            : intelCompose
            ? "Optional Intel XPU Docker helpers. NVIDIA and AMD should use Ollama, llama.cpp, or a CUDA/ROCm server on loopback instead of Start."
            : "Start/Download stay hidden unless your computer has a discrete Intel GPU and Docker (or LATE_VLLM_FORCE=1). NVIDIA, AMD, and CPU users: pick llama.cpp or Ollama, or point vLLM at a server you started."}
        </p>
        <div className="row" style={{ marginBottom: 8 }}>
          <span className="meta" style={{ flex: 1 }}>Models</span>
        </div>
        <div style={{ marginBottom: 6 }}>
          GPU: {gpu.downloading ? "downloading…" : gpu.starting ? "starting…" : gpu.running ? "running" : "stopped"}
          {gpu.models.length ? ` · ${gpu.models.join(", ")}` : ""}
        </div>
        <div className="meta" style={{ marginBottom: 8 }}>{gpu.detail}</div>
        {gpu.gpu?.summary && (
          <p className="meta" style={{ margin: "0 0 8px" }}>{gpu.gpu.summary}</p>
        )}
        <UseAllGpusRow
          settings={settings}
          gpu={gpu}
          engine="vllm"
          disabled={gpuBusy || gpu.running || gpu.starting}
          onChange={(next) => void setUseAllGpus(next)}
        />
        {showVllmStart && (
        <>
        {(gpu.localModels ?? []).length > 0 && (
          <div className="model-list">
            <span className="meta">Catalog for your computer</span>
            {(gpu.localModels ?? []).map((m) => (
              <div key={`rec-${m.id}`} className="model-line">
                <button
                  type="button"
                  className={serveModel === m.id ? "primary" : "ghost"}
                  title={m.note}
                  onClick={() => pickModel(m.id)}
                >
                  {m.complete ? "ready" : "get"} · {catalogFitLabel(m)} · {m.id}
                  {m.tp ? ` · TP${m.tp}` : ""}
                </button>
                {!m.complete && (
                  <button
                    type="button"
                    className="ghost dl"
                    disabled={gpuBusy || gpu.downloading}
                    onClick={() => void fetchModel(m.id)}
                  >
                    Download
                  </button>
                )}
              </div>
            ))}
          </div>
        )}
        {(gpu.localModels ?? []).some((m) => m.complete) && (
          <div className="model-list">
            <span className="meta">Downloaded on your computer</span>
            {(gpu.localModels ?? []).filter((m) => m.complete).map((m) => (
              <div key={`disk-${m.id}`} className="model-line">
                <button
                  type="button"
                  className={serveModel === m.id ? "primary" : "ghost"}
                  title={m.note}
                  onClick={() => pickModel(m.id)}
                >
                  on disk · {m.id}
                  {m.sizeBytes ? ` · ${fmtSize(m.sizeBytes)}` : ""}
                </button>
              </div>
            ))}
          </div>
        )}
        <label style={{ marginBottom: 8 }}>
          Weights to serve (Hugging Face id loaded into the GPUs)
          <select
            value={serveModel}
            onChange={(e) => {
              const id = e.target.value;
              setServeModel(id);
              try {
                localStorage.setItem("late.serveModel", id);
              } catch {
                /* ignore */
              }
            }}
          >
            {((gpu.localModels ?? []).length
              ? gpu.localModels
              : [{ id: serveModel || "Qwen/Qwen3-8B", complete: false, sizeBytes: 0, note: "" }]
            ).map((m) => (
              <option key={m.id} value={m.id}>
                {m.recommended ? "fits" : m.complete ? "cached" : "hub"}{m.newest ? " · newest" : ""} · {m.id}
                {m.sizeBytes ? ` (${fmtSize(m.sizeBytes)})` : ""}
              </option>
            ))}
          </select>
        </label>
        <p className="meta" style={{ margin: "0 0 8px" }}>
          {(gpu.localModels ?? []).find((m) => m.id === serveModel)?.note || "Pick a cached Hub id, or download one below."}
        </p>
        <div className="row" style={{ marginBottom: 8 }}>
          <input
            value={downloadId}
            placeholder="google/gemma-4-E4B-it or Qwen/Qwen3-8B"
            onChange={(e) => setDownloadId(e.target.value)}
          />
          <button
            className="ghost"
            disabled={gpuBusy || gpu.downloading || !downloadId.trim()}
            onClick={() => {
              const id = downloadId.trim();
              setGpuBusy(true);
              setServeModel(id);
              try {
                localStorage.setItem("late.serveModel", id);
              } catch {
                /* ignore */
              }
              void downloadInference(id)
                .then((s) => setGpu(fromStatus(s)))
                .catch((e: unknown) => setGpu((g) => ({ ...g, detail: e instanceof Error ? e.message : String(e) })))
                .finally(() => setGpuBusy(false));
            }}
          >
            Download
          </button>
        </div>
        <div className="row">
          <button
            className="primary"
            disabled={gpuBusy || gpu.running || gpu.starting || !serveModel.trim()}
            onClick={() => {
              const id = serveModel.trim();
              if (!id) {
                setGpu((g) => ({ ...g, detail: "pick Hugging Face weights above, then Start" }));
                return;
              }
              setGpuBusy(true);
              void startInference(id)
                .then((s) => setGpu(fromStatus(s)))
                .catch((e: unknown) => setGpu((g) => ({ ...g, detail: e instanceof Error ? e.message : String(e) })))
                .finally(() => setGpuBusy(false));
            }}
          >
            Start {serveModel ? serveModel.split("/").pop() : "local GPU"}
          </button>
          <button
            className="ghost"
            disabled={gpuBusy || (!gpu.running && !gpu.starting)}
            onClick={() => {
              setGpuBusy(true);
              void stopInference()
                .then((s) => setGpu(fromStatus(s)))
                .catch((e: unknown) => setGpu((g) => ({ ...g, detail: e instanceof Error ? e.message : String(e) })))
                .finally(() => setGpuBusy(false));
            }}
          >
            Stop
          </button>
        </div>
        </>
        )}
      </div>
      </ModelsDock>
      )}
      {backend === "mcp" && (
        <p className="hint" style={{ padding: "0 8px" }}>
          Chat goes through MCP (not Late’s vLLM on :8000). Device CLI still needs Approve. You start
          that program; Late will not start it. Paste the <code>/mcp</code> URL from the GUI
          (Copy MCP URL) — same port as the web UI, or <code>npm run mcp:http</code>. Drop a folder to
          Approve-grant it on the MCP host (same path must exist there; Late does not copy trees).
        </p>
      )}
      {backend === "cursor" && (
        <p className="hint" style={{ padding: "0 8px" }}>
          Add a Cursor key in the app: click <button className="linkish" type="button" onClick={() => setState({ keysOpen: true })}>API keys</button>
          {" "}(paste or import a file). Env <code>CURSOR_API_KEY</code> still overrides if set.
        </p>
      )}
      {(backend === "anthropic" || backend === "gemini" || backend === "azure") && (
        <p className="hint" style={{ padding: "0 8px" }}>
          {backend === "anthropic"
            ? "Anthropic key and base URL are in Settings. Public api.anthropic.com needs Cloud AI; a LAN proxy does not."
            : backend === "gemini"
              ? "Gemini key and base URL are in Settings. Public generativelanguage.googleapis.com needs Cloud AI; a private generateContent URL does not. Gemini is API-key generateContent, not Vertex OAuth."
              : "Azure key, resource URL, and deployment are in Settings. Public *.openai.azure.com needs Cloud AI unless DNS is only private."}{" "}
          <button className="linkish" type="button" onClick={() => setState({ keysOpen: true })}>API keys</button>
        </p>
      )}
      {(backend === "llamacpp" || backend === "ollama") && (
        <ModelsDock
          label={backend === "llamacpp" ? "llama.cpp" : "Ollama"}
          status={gpu.downloading ? (backend === "llamacpp" ? "downloading" : "pulling") : gpu.starting ? "starting" : gpu.running ? "running" : "stopped"}
        >
        <WeightsPanel
          engine={backend}
          gpu={gpu}
          serveModel={serveModel}
          downloadId={downloadId}
          gpuBusy={gpuBusy}
          sidecarHint={backend === "ollama" ? models.ollamaMessage : models.llamacppMessage}
          networkServer={targetOf(models, backend)?.egress !== "loopback"}
          onPick={pickModel}
          onDownloadId={setDownloadId}
          onFetch={(id) => void fetchModel(id, backend)}
          onStart={() => {
            if (backend === "ollama") {
              setGpuBusy(true);
              void startInference("serve", "ollama")
                .then((s) => setGpu(fromStatus(s)))
                .catch((e: unknown) => setGpu((g) => ({ ...g, detail: e instanceof Error ? e.message : String(e) })))
                .finally(() => setGpuBusy(false));
              return;
            }
            const id = serveModel.trim();
            if (!id) {
              setGpu((g) => ({ ...g, detail: "pick a GGUF above, then Start" }));
              return;
            }
            setGpuBusy(true);
            void startInference(id, "llamacpp")
              .then((s) => setGpu(fromStatus(s)))
              .catch((e: unknown) => setGpu((g) => ({ ...g, detail: e instanceof Error ? e.message : String(e) })))
              .finally(() => setGpuBusy(false));
          }}
          onStop={() => {
            setGpuBusy(true);
            void stopInference(backend === "ollama" ? "ollama" : "llamacpp")
              .then((s) => setGpu(fromStatus(s)))
              .catch((e: unknown) => setGpu((g) => ({ ...g, detail: e instanceof Error ? e.message : String(e) })))
              .finally(() => setGpuBusy(false));
          }}
          onUseAllGpus={(next) => void setUseAllGpus(next)}
          settings={settings}
          onPull={() => {
            const id = downloadId.trim();
            setGpuBusy(true);
            setServeModel(id);
            try {
              localStorage.setItem(serveStorageKey(backend), id);
            } catch {
              /* ignore */
            }
            void downloadInference(id, backend)
              .then((s) => {
                setGpu(fromStatus(s));
                if (backend === "ollama") setModel(id);
              })
              .catch((e: unknown) => setGpu((g) => ({ ...g, detail: e instanceof Error ? e.message : String(e) })))
              .finally(() => setGpuBusy(false));
          }}
        />
        </ModelsDock>
      )}
      <div className="log">
        {(messages ?? []).map((m) => (
          <div key={m.id} className={`msg ${m.role}${m.skipped ? " skipped" : ""}`}>
            <div className="msg-head">
              {m.logoDataUrl ? (
                <img className="msg-avatar" alt="" src={m.logoDataUrl} />
              ) : m.role === "assistant" && (m.nickname || m.label) ? (
                <span className="msg-avatar letter" aria-hidden>
                  {String(m.nickname || m.label || "?").trim().charAt(0).toUpperCase()}
                </span>
              ) : null}
              <div className="meta">{m.nickname || m.label || m.role}</div>
            </div>
            {m.content}
          </div>
        ))}
        {approval && (
          <div className="banner">
            Agent proposed a command to {approval.detail?.intent === "remediate" ? "implement a fix" : "answer your question"} — Approve to run it
          </div>
        )}
        {busy && (
          <div className="msg assistant thinking" aria-live="polite">
            <div className="meta">assistant</div>
            <div className="think-row">
              <span className="think-dots" aria-hidden>
                <i />
                <i />
                <i />
              </span>
              <span>
                {thinking || "Thinking"}
                {thinkMs >= 1000 ? ` · ${Math.floor(thinkMs / 1000)}s` : ""}
              </span>
            </div>
          </div>
        )}
      </div>
      {attached.length > 0 && (
        <div className="meta" style={{ padding: "4px 8px" }}>
          Ask by name: {attached.map((s) => s.name).join(", ")}
        </div>
      )}
      <div className="composer">
        <textarea
          value={input}
          placeholder="Ask a question — the agent will propose the commands needed (Approve to run)…"
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              if (!approval) void send();
            }
          }}
        />
        <button className="primary" disabled={busy} onClick={() => void send()}>
          Send
        </button>
        <button
          className="ghost"
          onClick={() => {
            queuedGrantRef.current = null;
            abort.current?.abort();
            void stopChat(conv.current);
            setState({ approval: null });
          }}
        >
          Stop
        </button>
      </div>
    </aside>
  );
}

function UseAllGpusRow(props: {
  settings: AppSettings | null;
  gpu: ReturnType<typeof fromStatus>;
  engine: "vllm" | "llamacpp" | "ollama";
  disabled?: boolean;
  onChange: (next: boolean) => void;
}) {
  const n = props.gpu.gpu?.discreteCount ?? props.gpu.gpuLaunch?.deviceCount ?? 0;
  const multi = Boolean(props.gpu.gpuLaunch?.multiVisible) || n >= 2;
  if (props.engine === "ollama") {
    return (
      <p className="meta" style={{ margin: "0 0 8px" }}>
        Ollama already uses every GPU it can see. Late does not pin it.
      </p>
    );
  }
  if (!multi) return null;
  const checked = props.settings?.use_all_gpus !== false;
  const note = props.gpu.gpuLaunch?.note;
  return (
    <div style={{ margin: "0 0 8px" }}>
      <label className="row" style={{ gap: 8, alignItems: "center" }}>
        <input
          type="checkbox"
          checked={checked}
          disabled={props.disabled}
          onChange={(e) => props.onChange(e.target.checked)}
        />
        <span>Use all GPUs on this computer ({n} found)</span>
      </label>
      {note ? <p className="meta" style={{ margin: "4px 0 0" }}>{note}</p> : null}
    </div>
  );
}

function WeightsPanel(props: {
  engine: "llamacpp" | "ollama";
  gpu: ReturnType<typeof fromStatus>;
  serveModel: string;
  downloadId: string;
  gpuBusy: boolean;
  sidecarHint?: string;
  networkServer?: boolean;
  settings: AppSettings | null;
  onPick: (id: string) => void;
  onDownloadId: (id: string) => void;
  onFetch: (id: string) => void;
  onStart: () => void;
  onStop: () => void;
  onPull: () => void;
  onUseAllGpus: (next: boolean) => void;
}) {
  const llama = props.engine === "llamacpp";
  const statusWord = props.gpu.downloading
    ? llama
      ? "downloading…"
      : "pulling…"
    : props.gpu.starting
      ? "starting…"
      : props.gpu.running
        ? "running"
        : "stopped";
  return (
    <div className="chat-models-body">
      <p className="hint" style={{ margin: "0 0 8px" }}>
        {props.networkServer
          ? `${props.sidecarHint ? `${props.sidecarHint} ` : ""}Pull / Start / Download only manage a process on this computer, so they stay hidden while the URL is not loopback. Pick Local to use this computer.`
          : props.sidecarHint ||
            (llama
              ? "Download a GGUF from Hugging Face into ~/.local/share/late/models/gguf/. Start runs the bundled llama-server on 127.0.0.1:8080 (Vulkan on Linux/Windows, Metal on Mac)."
              : "Pull talks to Ollama on loopback (library names like gemma4:e4b or qwen3:8b, or Hugging Face ids like google/gemma-4-E4B-it-qat-q4_0-gguf). Start runs bundled ollama serve on 127.0.0.1:11434 if it is not already up.")}
      </p>
      <div className="row" style={{ marginBottom: 8 }}>
        <span className="meta" style={{ flex: 1 }}>{llama ? "GGUF" : "Ollama models"}</span>
      </div>
      <div style={{ marginBottom: 6 }}>
        {llama ? "llama.cpp" : "Ollama"}: {statusWord}
        {props.gpu.models.length ? ` · ${props.gpu.models.join(", ")}` : ""}
      </div>
      <div className="meta" style={{ marginBottom: 8 }}>{props.gpu.detail}</div>
      {!props.networkServer && (
      <UseAllGpusRow
        settings={props.settings}
        gpu={props.gpu}
        engine={props.engine}
        disabled={props.gpuBusy || props.gpu.running || props.gpu.starting}
        onChange={props.onUseAllGpus}
      />
      )}
      {(props.gpu.localModels ?? []).length > 0 && (
        <div className="model-list">
          <span className="meta">Catalog for your computer</span>
          {(props.gpu.localModels ?? []).map((m) => (
            <div key={`rec-${m.id}`} className="model-line">
              <button
                type="button"
                className={props.serveModel === m.id ? "primary" : "ghost"}
                title={m.note}
                onClick={() => props.onPick(m.id)}
              >
                {m.complete ? "ready" : "get"} · {catalogFitLabel(m)} · {m.id}
              </button>
              {!m.complete && !props.networkServer && (
                <button
                  type="button"
                  className="ghost dl"
                  disabled={props.gpuBusy || props.gpu.downloading}
                  onClick={() => props.onFetch(m.id)}
                >
                  {llama ? "Download" : "Pull"}
                </button>
              )}
            </div>
          ))}
        </div>
      )}
      {(props.gpu.localModels ?? []).some((m) => m.complete) && (
        <div className="model-list">
          <span className="meta">{llama ? "Downloaded on your computer" : "Pulled into Ollama"}</span>
          {(props.gpu.localModels ?? []).filter((m) => m.complete).map((m) => (
            <div key={`disk-${m.id}`} className="model-line">
              <button
                type="button"
                className={props.serveModel === m.id ? "primary" : "ghost"}
                title={m.note}
                onClick={() => props.onPick(m.id)}
              >
                {llama ? "on disk" : "pulled"} · {m.id}
                {m.sizeBytes ? ` · ${fmtSize(m.sizeBytes)}` : ""}
              </button>
            </div>
          ))}
        </div>
      )}
      <label style={{ marginBottom: 8 }}>
        {llama ? "GGUF to serve (Hugging Face id)" : "Model to pull (Ollama name or Hugging Face id)"}
        <select
          value={props.serveModel}
          onChange={(e) => props.onPick(e.target.value)}
        >
          {((props.gpu.localModels ?? []).length
            ? props.gpu.localModels
            : [{ id: props.serveModel || defaultHubId(props.engine), complete: false, sizeBytes: 0, note: "" }]
          ).map((m) => (
            <option key={m.id} value={m.id}>
              {m.recommended ? "fits" : m.complete ? "ready" : "hub"}{m.newest ? " · newest" : ""} · {m.id}
              {m.sizeBytes ? ` (${fmtSize(m.sizeBytes)})` : ""}
            </option>
          ))}
        </select>
      </label>
      <p className="meta" style={{ margin: "0 0 8px" }}>
        {(props.gpu.localModels ?? []).find((m) => m.id === props.serveModel)?.note
          || (props.networkServer
            ? "Chat uses the saved server. Start / Pull / Download stay on this computer."
            : llama
              ? "Pick a GGUF, or download one below."
              : "Pick a pulled model, or Pull one below.")}
      </p>
      {!props.networkServer && (
      <div className="row" style={{ marginBottom: 8 }}>
        <input
          value={props.downloadId}
          placeholder={llama ? "Qwen/Qwen3-8B-GGUF or google/gemma-4-E4B-it-qat-q4_0-gguf" : "gemma4:e4b or Qwen/Qwen3-8B-GGUF"}
          onChange={(e) => props.onDownloadId(e.target.value)}
        />
        <button
          className="ghost"
          disabled={props.gpuBusy || props.gpu.downloading || !props.downloadId.trim()}
          onClick={props.onPull}
        >
          {llama ? "Download" : "Pull"}
        </button>
      </div>
      )}
      {!props.networkServer && (
        <div className="row">
          <button
            className="primary"
            disabled={
              props.gpuBusy ||
              props.gpu.running ||
              props.gpu.starting ||
              (llama && !props.serveModel.trim())
            }
            onClick={props.onStart}
          >
            {llama
              ? `Start ${props.serveModel ? props.serveModel.split("/").pop() : "llama-server"}`
              : "Start Ollama"}
          </button>
          <button
            className="ghost"
            disabled={props.gpuBusy || (!props.gpu.lateOwned && !props.gpu.starting)}
            onClick={props.onStop}
          >
            Stop
          </button>
        </div>
      )}
    </div>
  );
}

function ModelsDock(props: { label: string; status: string; children: ReactNode }) {
  const h = useApp((s) => s.chatModelsH);
  const lastH = useRef(h > 0 ? h : MODELS_H.fallback);
  useEffect(() => {
    if (h > 0) lastH.current = h;
  }, [h]);
  const open = h > 0;

  function onGutterDown(e: MouseEvent<HTMLDivElement>) {
    e.preventDefault();
    const startY = e.clientY;
    const startH = h > 0 ? h : lastH.current;
    const move = (ev: globalThis.MouseEvent) =>
      setChatModelsH(Math.max(MODELS_H.min, startH + (ev.clientY - startY)));
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  }

  return (
    <div className={`chat-models-dock${open ? "" : " collapsed"}`}>
      <div className="chat-models-bar">
        <span className="meta">
          {props.label}: {props.status}
        </span>
        <button
          type="button"
          className="ghost"
          onClick={() => setChatModelsH(open ? 0 : lastH.current || MODELS_H.fallback)}
        >
          {open ? "Hide" : "Show"}
        </button>
      </div>
      {open && (
        <>
          <div className="chat-models" style={{ height: h }}>
            {props.children}
          </div>
          <div
            className="chat-models-gutter"
            title="Drag to resize models vs chat"
            onMouseDown={onGutterDown}
          />
        </>
      )}
    </div>
  );
}
