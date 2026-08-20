import { useEffect, useRef, useState, type MouseEvent, type ReactNode } from "react";
import { isProposalDismissed } from "../lib/approvals-ui";
import { sidecarHealth, sidecarModels, sidecarPending, stopChat, streamChat, type SidecarModels } from "../lib/sidecar";
import {
  downloadInference,
  inferenceStatus,
  dockAgent,
  setChatModelsH,
  setState,
  startInference,
  stopInference,
  useApp,
  widenChat,
  type InferenceStatus,
  type LocalModel,
} from "../store";
import { onShellDragEnd, onShellDragStart } from "../shellDnD";
import { isAgentStacked, MODELS_H } from "../shell";
import { backendLabel, coerceChatBackend, newId, type ChatBackend, type ChatMsg } from "../types";

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
  };
}

function serveStorageKey(backend: ChatBackend) {
  if (backend === "llamacpp") return "late.ggufServe";
  if (backend === "ollama") return "late.ollamaServe";
  return "late.serveModel";
}

function defaultHubId(backend: ChatBackend) {
  if (backend === "llamacpp") return "Qwen/Qwen3-8B-GGUF";
  if (backend === "ollama") return "qwen2.5:7b";
  return "Qwen/Qwen3-8B";
}

function inferenceEngine(backend: ChatBackend): "vllm" | "llamacpp" | "ollama" | null {
  if (backend === "llamacpp") return "llamacpp";
  if (backend === "ollama") return "ollama";
  if (backend === "local") return "vllm";
  return null;
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
    backends: ["local", "llamacpp", "ollama", "cursor"],
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
    allowIntelCompose: false,
    lateOwned: false,
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
      if (b === "llamacpp") return m.llamacpp[0] || "";
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
            const rec = (s.localModels ?? []).find((m) => m.recommended);
            const complete = (s.localModels ?? []).find((m) => m.complete && m.recommended);
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
    else if (backendNext === "cursor") setModel(settings.cursor_model || models.cursor[0] || "composer-2.5");
    else setModel(settings.vllm_model || models.local[0] || "local");
  }, [settings, models.cursor, models.local, models.ollama, models.llamacpp]);

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
    if (!text || busy) return;
    if (backend === "cursor" && !settings?.cloud_chat_enabled) {
      setStatus("Cloud AI is off. Turn it on in Settings to use Cursor.");
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
    setBusy(true);
    setThinking("Thinking");
    setState({ approval: null });
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

  const intelCompose = gpu.allowIntelCompose;

  return (
    <aside className="chat">
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
        <span className="meta">{status}</span>
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
              return;
            }
            setBackend(next);
            resetChat({ backend: next, label: backendLabel(next) });
          }}
        >
          <option value="local">local vLLM</option>
          <option value="llamacpp">llama.cpp</option>
          <option value="ollama">Ollama</option>
          <option value="cursor" disabled={!settings?.cloud_chat_enabled}>
            Cursor SDK{settings?.cloud_chat_enabled ? "" : " — Cloud AI is off"}
          </option>
        </select>
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
              : backend === "local"
                ? (models.local.length ? models.local : ["local"])
                : (models.cursor.length ? models.cursor : ["composer-2.5", "auto"])
          ).map((m) => (
            <option key={m} value={m} disabled={m.startsWith("(")}>{m}</option>
          ))}
        </select>
      </div>
      {backend === "local" && (
      <ModelsDock
        label="vLLM"
        status={gpu.downloading ? "downloading" : gpu.starting ? "starting" : gpu.running ? "running" : "stopped"}
      >
      <div className="chat-models-body">
        <p className="hint" style={{ margin: "0 0 8px" }}>
          {intelCompose
            ? "Optional Intel XPU Docker helpers. NVIDIA and AMD should use Ollama, llama.cpp, or a CUDA/ROCm server on loopback instead of Start."
            : "Start/Download stay hidden unless your computer has a discrete Intel GPU (or LATE_VLLM_FORCE=1). NVIDIA, AMD, and CPU users: pick llama.cpp or Ollama, or point Local at a server you started."}
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
        {intelCompose && (
        <>
        {(gpu.localModels ?? []).some((m) => m.recommended) && (
          <div className="model-list">
            <span className="meta">Recommended for your computer</span>
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
            placeholder="google/gemma-3-4b-it or Qwen/Qwen3-8B"
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
      {backend === "cursor" && (
        <p className="hint" style={{ padding: "0 8px" }}>
          Add a Cursor key in the app: click <button className="linkish" type="button" onClick={() => setState({ keysOpen: true })}>API keys</button>
          {" "}(paste or import a file). Env <code>CURSOR_API_KEY</code> still overrides if set.
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
          onPick={pickModel}
          onDownloadId={setDownloadId}
          onFetch={(id) => void fetchModel(id, backend)}
          onStart={() => {
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
            void stopInference("llamacpp")
              .then((s) => setGpu(fromStatus(s)))
              .catch((e: unknown) => setGpu((g) => ({ ...g, detail: e instanceof Error ? e.message : String(e) })))
              .finally(() => setGpuBusy(false));
          }}
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
            setState({ approval: null });
          }}
        >
          Stop
        </button>
      </div>
    </aside>
  );
}

function WeightsPanel(props: {
  engine: "llamacpp" | "ollama";
  gpu: ReturnType<typeof fromStatus>;
  serveModel: string;
  downloadId: string;
  gpuBusy: boolean;
  sidecarHint?: string;
  onPick: (id: string) => void;
  onDownloadId: (id: string) => void;
  onFetch: (id: string) => void;
  onStart: () => void;
  onStop: () => void;
  onPull: () => void;
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
        {props.sidecarHint ||
          (llama
            ? "Download a GGUF from Hugging Face into ~/.local/share/late/models/gguf/. Late does not install llama.cpp. Start needs llama-server on PATH, or run it yourself on 127.0.0.1:8080."
            : "Pull talks to Ollama on loopback (library names like gemma3:4b or qwen2.5:7b, or Hugging Face ids like google/gemma-3-4b-it-qat-q4_0-gguf). Late does not install Ollama.")}
      </p>
      <div className="row" style={{ marginBottom: 8 }}>
        <span className="meta" style={{ flex: 1 }}>{llama ? "GGUF" : "Ollama models"}</span>
      </div>
      <div style={{ marginBottom: 6 }}>
        {llama ? "llama.cpp" : "Ollama"}: {statusWord}
        {props.gpu.models.length ? ` · ${props.gpu.models.join(", ")}` : ""}
      </div>
      <div className="meta" style={{ marginBottom: 8 }}>{props.gpu.detail}</div>
      {(props.gpu.localModels ?? []).some((m) => m.recommended) && (
        <div className="model-list">
          <span className="meta">Recommended for your computer</span>
          {(props.gpu.localModels ?? []).filter((m) => m.recommended).map((m) => (
            <div key={`rec-${m.id}`} className="model-line">
              <button
                type="button"
                className={props.serveModel === m.id ? "primary" : "ghost"}
                title={m.note}
                onClick={() => props.onPick(m.id)}
              >
                {m.complete ? "ready" : "get"} · {m.id}
              </button>
              {!m.complete && (
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
              {m.recommended ? "fits" : m.complete ? "ready" : "hub"} · {m.id}
              {m.sizeBytes ? ` (${fmtSize(m.sizeBytes)})` : ""}
            </option>
          ))}
        </select>
      </label>
      <p className="meta" style={{ margin: "0 0 8px" }}>
        {(props.gpu.localModels ?? []).find((m) => m.id === props.serveModel)?.note
          || (llama ? "Pick a GGUF, or download one below." : "Pick a pulled model, or Pull one below.")}
      </p>
      <div className="row" style={{ marginBottom: 8 }}>
        <input
          value={props.downloadId}
          placeholder={llama ? "Qwen/Qwen3-8B-GGUF or google/gemma-3-4b-it-qat-q4_0-gguf" : "gemma3:4b or Qwen/Qwen3-8B-GGUF"}
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
      {llama && (
        <div className="row">
          <button
            className="primary"
            disabled={props.gpuBusy || props.gpu.running || props.gpu.starting || !props.serveModel.trim()}
            onClick={props.onStart}
          >
            Start {props.serveModel ? props.serveModel.split("/").pop() : "llama-server"}
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
