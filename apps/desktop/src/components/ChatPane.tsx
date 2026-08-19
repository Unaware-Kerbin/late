import { useEffect, useRef, useState } from "react";
import { sidecarHealth, sidecarModels, stopChat, streamChat, type SidecarModels } from "../lib/sidecar";
import {
  downloadInference,
  inferenceStatus,
  setState,
  startInference,
  stopInference,
  useApp,
  type InferenceStatus,
  type LocalModel,
} from "../store";
import { backendLabel, coerceChatBackend, newId, type ChatBackend, type ChatMsg } from "../types";

function fmtSize(n: number) {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)} GB`;
  if (n >= 1_000_000) return `${Math.round(n / 1_000_000)} MB`;
  if (n <= 0) return "";
  return `${n} B`;
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
  };
}

export function ChatPane() {
  const approval = useApp((s) => s.approval);
  const agentSeed = useApp((s) => s.agentSeed);
  const sessions = useApp((s) => s.sessions);
  const settings = useApp((s) => s.settings);
  const attached = (sessions ?? []).filter((s) => String(s.kind).toLowerCase() !== "sftp");
  const [messages, setMessages] = useState<ChatMsg[]>([]);
  const [input, setInput] = useState("");
  const [backend, setBackend] = useState<ChatBackend>("local");
  const [model, setModel] = useState("");
  const [models, setModels] = useState<SidecarModels>({
    local: [],
    cursor: [],
    ollama: [],
    backends: ["local", "cursor", "ollama"],
  });
  const [busy, setBusy] = useState(false);
  const [thinking, setThinking] = useState("");
  const [thinkMs, setThinkMs] = useState(0);
  const [status, setStatus] = useState("sidecar idle");
  const [gpu, setGpu] = useState({
    running: false,
    starting: false,
    downloading: false,
    models: [] as string[],
    localModels: [] as LocalModel[],
    gpu: undefined as InferenceStatus["gpu"],
    detail: "checking…",
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
  const [modelsExpanded, setModelsExpanded] = useState(false);
  const conv = useRef(newId());
  const abort = useRef<AbortController>();
  const backendRef = useRef(backend);
  backendRef.current = backend;
  const settingsApplied = useRef(false);

  async function refreshModels() {
    const m = await sidecarModels();
    setModels(m);
    setModel((cur) => {
      if (cur) return cur;
      const b = backendRef.current;
      if (b === "ollama") return m.ollama[0] || "";
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
    const tick = () => {
      void inferenceStatus()
        .then((s) => {
          setGpu(fromStatus(s));
          setServeModel((cur) => {
            const ids = (s.localModels ?? []).map((m) => m.id);
            if (cur && ids.includes(cur)) return cur;
            const rec = (s.localModels ?? []).find((m) => m.recommended);
            const complete = (s.localModels ?? []).find((m) => m.complete && m.recommended);
            const next = complete?.id || rec?.id || s.localModels?.[0]?.id || cur || "Qwen/Qwen3-8B";
            try {
              if (next) localStorage.setItem("late.serveModel", next);
            } catch {
              /* ignore */
            }
            return next;
          });
          if (s.running && s.models?.length) void refreshModels().catch(() => undefined);
          else if (backendRef.current === "ollama") void refreshModels().catch(() => undefined);
        })
        .catch(() =>
          setGpu((g) => ({
            ...g,
            detail: g.starting || g.running
              ? `${g.detail} (status poll delayed — sessions should stay live)`
              : "daemon inference API unavailable",
          })),
        );
    };
    tick();
    const id = window.setInterval(tick, 5000);
    return () => window.clearInterval(id);
  }, []);

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
    setMessages([]);
    setInput("");
    setBusy(false);
    setThinking("");
    setState({ approval: null });
    const b = opts?.backend ?? backend;
    if (opts?.backend) {
      if (b === "ollama") setModel(opts.model || models.ollama[0] || "");
      else if (b === "local") setModel(opts.model || models.local[0] || "local");
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
    setBackend(next);
    if (next === "ollama") setModel(settings.ollama_model || models.ollama[0] || "");
    else if (next === "cursor") setModel(settings.cursor_model || models.cursor[0] || "composer-2.5");
    else setModel(settings.vllm_model || models.local[0] || "local");
  }, [settings, models.cursor, models.local, models.ollama]);

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
    if (!text || busy) return;
    const next: ChatMsg[] = [...messages, { id: newId(), role: "user", content: text }];
    setMessages(next);
    setInput("");
    setBusy(true);
    setThinking("Thinking");
    abort.current = new AbortController();
    let assistant = "";
    const id = newId();
    try {
      await streamChat({
        messages: next,
        backend,
        model: model || undefined,
        conversationId: conv.current,
        signal: abort.current.signal,
        onEvent: (e) => {
          if (e.type === "delta") {
            assistant += e.text;
            setThinking("Writing");
            setMessages([...next, { id, role: "assistant", content: assistant }]);
          } else if (e.type === "tool") {
            const name = e.name || "tool";
            if (e.status === "start") setThinking(`Using ${name.replace(/_/g, " ")}`);
            else if (e.status === "ok") setThinking("Thinking about the result");
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
            setThinking("Waiting for you to Approve");
            setState({ approval: e.pending });
          } else if (e.type === "ask") {
            setThinking("Waiting for your answer");
            setState({
              approval: {
                proposalId: e.proposalId,
                kind: "ask",
                title: "Agent question",
                detail: { question: e.question },
              },
            });
          } else if (e.type === "error") {
            setStatus(e.message);
            setThinking("");
            setMessages((cur) => [
              ...cur,
              { id: newId(), role: "system", content: `Agent stopped: ${e.message}` },
            ]);
          } else if (e.type === "round") {
            setThinking(`Thinking · round ${e.n}/${e.max}`);
            setStatus(`round ${e.n}/${e.max}`);
          } else if (e.type === "heartbeat") {
            setThinking(e.message || "Still thinking");
            setStatus(e.message || "still thinking");
          } else if (e.type === "done") {
            setThinking("");
            if (!assistant) setMessages([...next, { id, role: "assistant", content: e.message }]);
          }
        },
      });
    } catch (err) {
      setStatus(err instanceof Error ? err.message : String(err));
      setThinking("");
    } finally {
      setBusy(false);
      setThinking("");
    }
  }

  function pickModel(id: string) {
    setServeModel(id);
    setDownloadId(id);
    try {
      localStorage.setItem("late.serveModel", id);
    } catch {
      /* ignore */
    }
  }

  async function fetchModel(id: string) {
    pickModel(id);
    setGpuBusy(true);
    try {
      setGpu(fromStatus(await downloadInference(id)));
    } catch (e: unknown) {
      setGpu((g) => ({ ...g, detail: e instanceof Error ? e.message : String(e) }));
    } finally {
      setGpuBusy(false);
    }
  }

  return (
    <aside className="chat">
      <div className="chat-head">
        Agent
        <span className="meta">{status}</span>
      </div>
      <div className="row" style={{ padding: 8 }}>
        <select
          value={backend}
          onChange={(e) => {
            const next = coerceChatBackend(e.target.value);
            setBackend(next);
            resetChat({ backend: next, label: backendLabel(next) });
          }}
        >
          <option value="local">local vLLM</option>
          <option value="ollama">Ollama</option>
          <option value="cursor">Cursor SDK</option>
        </select>
        <select
          title="Chat API model id after the server is up (usually “local”)"
          value={model}
          onChange={(e) => {
            const next = e.target.value;
            if (next !== model) resetChat({ model: next, label: next });
          }}
        >
          {(backend === "ollama"
            ? (models.ollama.length ? models.ollama : ["(no Ollama models)"])
            : backend === "local"
              ? (models.local.length ? models.local : ["local"])
              : (models.cursor.length ? models.cursor : ["composer-2.5", "auto"])
          ).map((m) => (
            <option key={m} value={m} disabled={m.startsWith("(")}>{m}</option>
          ))}
        </select>
      </div>
      {backend === "local" && (
      <div className={`chat-models${modelsExpanded ? " expanded" : ""}`}>
        <div className="row" style={{ marginBottom: 8 }}>
          <span className="meta" style={{ flex: 1 }}>Models</span>
          <button
            type="button"
            className="ghost"
            onClick={() => setModelsExpanded((v) => !v)}
          >
            {modelsExpanded ? "Collapse" : "Expand"}
          </button>
        </div>
        <div style={{ marginBottom: 6 }}>
          GPU vLLM: {gpu.downloading ? "downloading…" : gpu.starting ? "starting…" : gpu.running ? "running" : "stopped"}
          {gpu.models.length ? ` · ${gpu.models.join(", ")}` : ""}
        </div>
        <div className="meta" style={{ marginBottom: 8 }}>{gpu.detail}</div>
        {gpu.gpu?.summary && (
          <p className="meta" style={{ margin: "0 0 8px" }}>{gpu.gpu.summary}</p>
        )}
        {(gpu.localModels ?? []).some((m) => m.recommended) && (
          <div className="model-list">
            <span className="meta">Recommended for this machine</span>
            {(gpu.localModels ?? []).filter((m) => m.recommended).map((m) => (
              <div key={`rec-${m.id}`} className="model-line">
                <button
                  type="button"
                  className={serveModel === m.id ? "primary" : "ghost"}
                  title={m.note}
                  onClick={() => pickModel(m.id)}
                >
                  {m.complete ? "ready" : "get"} · {m.id}
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
            <span className="meta">Downloaded on this machine</span>
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
                {m.recommended ? "fits" : m.complete ? "cached" : "hub"} · {m.id}
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
            placeholder="Qwen/Qwen3-8B"
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
      </div>
      )}
      {backend === "cursor" && (
        <p className="hint" style={{ padding: "0 8px" }}>
          Add a Cursor key in the app: click <button className="linkish" type="button" onClick={() => setState({ keysOpen: true })}>API keys</button>
          {" "}(paste or import a file). Env <code>CURSOR_API_KEY</code> still overrides if set.
        </p>
      )}
      {backend === "ollama" && (
        <p className="hint" style={{ padding: "0 8px" }}>
          {models.ollamaMessage ||
            "Ollama is optional. Install from https://ollama.com, run `ollama pull <model>`, then select a model here. Late does not install Ollama or download weights."}
        </p>
      )}
      <div className="log">
        {(messages ?? []).map((m) => (
          <div key={m.id} className={`msg ${m.role}`}>
            <div className="meta">{m.role}</div>
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
            abort.current?.abort();
            void stopChat(conv.current);
          }}
        >
          Stop
        </button>
      </div>
    </aside>
  );
}
