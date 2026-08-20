import { useEffect, useState } from "react";
import { dismissProposal } from "../lib/approvals-ui";
import { approve } from "../lib/sidecar";
import {
  DENSITIES,
  FONTS,
  LAYOUTS,
  RADII,
  THEMES,
} from "../appearance";
import {
  deleteAuth,
  deleteCollection,
  diffCaptures,
  importFile,
  closeTab,
  getState,
  newTab,
  openLocal,
  openPcapPane,
  openSession,
  saveSettings,
  saveProviderKey,
  clearProviderKey,
  setAppearance,
  patchAppearance,
  runCollection,
  setState,
  toast,
  splitFocused,
  bumpTermFont,
  bumpUiScale,
  resetAppearance,
  startDeviceEditor,
  upsertAuth,
  upsertCollection,
  saveDeviceWithLogin,
  upsertDevice,
  useApp,
} from "../store";
import { rpc } from "../lib/rpc";
import { VENDORS, coerceChatBackend, emptyDevice, newId, normalizeFolderPath, type AuthProfile, type Device, type DeviceKind } from "../types";

function trap(e: KeyboardEvent) {
  if (e.key === "Escape") {
    e.preventDefault();
    e.stopPropagation();
    return;
  }
  if (e.key !== "Enter") return;
  const el = e.target as HTMLElement | null;
  // Newlines in the ask-user box are fine; Enter must never Approve / submit.
  if (el?.tagName === "TEXTAREA" && el.closest(".modal")) {
    e.stopPropagation();
    return;
  }
  e.preventDefault();
  e.stopPropagation();
}

export function Modals() {
  const hostKey = useApp((s) => s.hostKey);
  const approval = useApp((s) => s.approval);
  const paletteOpen = useApp((s) => s.paletteOpen);
  const importOpen = useApp((s) => s.importOpen);
  const settingsOpen = useApp((s) => s.settingsOpen);
  const keysOpen = useApp((s) => s.keysOpen);
  const collectionsOpen = useApp((s) => s.collectionsOpen);
  const captureOpen = useApp((s) => s.captureOpen);
  const authOpen = useApp((s) => s.authOpen);
  const deviceEditor = useApp((s) => s.deviceEditor);
  return (
    <>
      {hostKey && <HostKeyModal />}
      {approval && <ApprovalModal key={approval.proposalId} />}
      {paletteOpen && <Palette />}
      {importOpen && <ImportModal />}
      {settingsOpen && <SettingsModal />}
      {keysOpen && <KeysModal />}
      {collectionsOpen && <CollectionsModal />}
      {captureOpen && <CaptureModal />}
      {authOpen && <AuthModal />}
      {deviceEditor && <DeviceModal key={deviceEditor.id} device={deviceEditor} />}
    </>
  );
}

function HostKeyModal() {
  const hostKey = useApp((s) => s.hostKey)!;
  const [busy, setBusy] = useState(false);
  async function accept() {
    if (busy) return;
    setBusy(true);
    const retry = hostKey.retry;
    setState({ hostKey: null });
    try {
      await retry();
    } catch (err) {
      toast("error", err instanceof Error ? err.message : String(err));
    }
  }
  function cancel() {
    if (busy) return;
    setState({ hostKey: null });
  }
  useEffect(() => {
    window.addEventListener("keydown", trap, true);
    return () => window.removeEventListener("keydown", trap, true);
  }, []);
  return (
    <div className="modal-root hostkey" onMouseDown={(e) => e.stopPropagation()}>
      <div className="modal" role="dialog" aria-modal="true">
        <h2>{hostKey.mismatch ? "Host key mismatch" : "Unknown host key"}</h2>
        <p>
          {hostKey.mismatch
            ? "The presented key does not match the pin. An attacker could be intercepting this connection."
            : "This host has never been seen. Trust it only if the fingerprint matches what you expect."}
        </p>
        <div className="fp">host {hostKey.host}</div>
        {hostKey.pinned && <div className="fp">pinned {hostKey.pinned}</div>}
        <div className="fp">presented {hostKey.presented}</div>
        <div className="actions">
          <button type="button" className="ghost" onClick={cancel} disabled={busy}>
            Cancel
          </button>
          <button type="button" className="primary" onClick={() => void accept()} disabled={busy}>
            {busy ? "Connecting…" : hostKey.mismatch ? "Replace pin and connect" : "Trust and connect"}
          </button>
        </div>
      </div>
    </div>
  );
}

function approvalSessionId(detail: Record<string, unknown>): string {
  const v = detail.sessionId ?? detail.session_id;
  return typeof v === "string" && v.trim() ? v : "";
}

function approvalPrimary(approval: {
  kind: string;
  expanded?: string;
  detail: Record<string, unknown>;
}): string {
  if (approval.kind === "ask" && typeof approval.detail.question === "string") {
    return approval.detail.question;
  }
  if (typeof approval.expanded === "string" && approval.expanded.trim()) return approval.expanded;
  const d = approval.detail;
  if (typeof d.expanded === "string" && d.expanded.trim()) return d.expanded;
  if (typeof d.command === "string" && d.command.trim()) return d.command;
  if (typeof d.path === "string") {
    const method = typeof d.method === "string" ? d.method : "GET";
    return `${method} ${d.path}`;
  }
  return JSON.stringify({ ...d, expanded: approval.expanded }, null, 2);
}

function ApprovalModal() {
  const approval = useApp((s) => s.approval)!;
  const sessions = useApp((s) => s.sessions);
  const panes = useApp((s) => s.panes);
  const focusedPaneId = useApp((s) => s.focusedPaneId);
  const linux = Boolean(approval.linuxUnrestricted);
  const detail = approval.detail ?? {};
  const [ack, setAck] = useState(false);
  const [always, setAlways] = useState(false);
  const [answer, setAnswer] = useState("");
  const [wrap, setWrap] = useState(true);
  useEffect(() => {
    window.addEventListener("keydown", trap, true);
    return () => window.removeEventListener("keydown", trap, true);
  }, []);
  const blocked = approval.policyAllowed === false;
  const canAlways = !linux && !blocked && approval.kind !== "ask" && approval.allowAlwaysAllow !== false;
  const primary = approvalPrimary({ ...approval, detail });
  const sessionId = approvalSessionId(detail);
  const target = sessions.find((s) => s.id === sessionId);
  const focused = focusedPaneId ? panes[focusedPaneId] : undefined;
  const mismatch = Boolean(sessionId && focused?.session?.id && focused.session.id !== sessionId);
  const proposed = typeof detail.command === "string" ? detail.command : "";
  const expanded =
    (typeof approval.expanded === "string" && approval.expanded) ||
    (typeof detail.expanded === "string" ? detail.expanded : "");
  async function decide(allow: boolean) {
    try {
      await approve(approval.proposalId, allow, allow ? { alwaysAllow: always && canAlways, answer } : undefined);
      dismissProposal(approval.proposalId);
      setState({ approval: null });
    } catch (err) {
      toast("error", err instanceof Error ? err.message : String(err));
    }
  }
  return (
    <div className="modal-root" onMouseDown={(e) => e.stopPropagation()}>
      <div className="modal" role="dialog" aria-modal="true">
        <h2>{approval.title}</h2>
        {linux && <div className="banner linux">Linux session — no permit list, no always-allow. Every command needs an explicit click.</div>}
        {blocked && (
          <div className="banner">
            Permit list denied: {approval.policyReason || "vendor permit list rejected this command"}
            <div className="hint" style={{ marginTop: 8, textTransform: "none", letterSpacing: 0 }}>
              This will not be sent. Copy the command below into the terminal if you want to apply it yourself. The agent should also print the rest of the ACL for you.
            </div>
          </div>
        )}
        {sessionId && approval.kind !== "ask" && (
          <p className="hint">
            Target: {target?.name ?? sessionId}
            {target ? ` (${target.kind})` : ""}
          </p>
        )}
        {mismatch && (
          <div className="banner">This command targets a different session than the focused terminal.</div>
        )}
        {typeof detail.reason === "string" && detail.reason && (
          <p className="hint">{String(detail.reason)}</p>
        )}
        {detail.intent === "remediate" && (
          <div className="banner">This is a suggested fix. It will change device state if you Approve.</div>
        )}
        {detail.intent === "investigate" && (
          <p className="hint">Read-only gather step so the agent can answer your question. Still requires Approve.</p>
        )}
        {proposed && expanded && proposed !== expanded && (
          <p className="hint">As proposed: {proposed}</p>
        )}
        <div className="fp-toolbar">
          <label className="check">
            <input type="checkbox" checked={wrap} onChange={(e) => setWrap(e.target.checked)} />
            Wrap text
          </label>
        </div>
        <pre className={wrap ? "fp wrap" : "fp nowrap"}>{primary}</pre>
        {approval.kind === "ask" && (
          <textarea value={answer} onChange={(e) => setAnswer(e.target.value)} placeholder="Your answer" />
        )}
        <label className="check">
          <input type="checkbox" checked={ack} onChange={(e) => setAck(e.target.checked)} />
          I reviewed the expanded command / request and want it sent.
        </label>
        {canAlways && (
          <label className="check">
            <input type="checkbox" checked={always} onChange={(e) => setAlways(e.target.checked)} />
            Always allow this expanded command in this chat (permit list still runs).
          </label>
        )}
        <div className="actions">
          <button
            type="button"
            className="ghost"
            onClick={() => void decide(false)}
          >
            Cancel
          </button>
          <button
            type="button"
            className="primary"
            disabled={!ack || blocked}
            onClick={() => void decide(true)}
          >
            Approve
          </button>
        </div>
      </div>
    </div>
  );
}

function Palette() {
  const [q, setQ] = useState("");
  const devices = useApp((s) => s.inventory.devices);
  const items = [
    { id: "local", label: "Open local PTY", run: () => void openLocal() },
    { id: "pcap", label: "Open packet capture", run: () => openPcapPane() },
    { id: "tab", label: "New tab", run: () => newTab() },
    { id: "tab-close", label: "Close tab", run: () => closeTab(getState().activeTabId ?? "") },
    { id: "split-r", label: "Split right", run: () => splitFocused("right") },
    { id: "split-d", label: "Split down", run: () => splitFocused("down") },
    { id: "import", label: "Import inventory", run: () => setState({ importOpen: true }) },
    { id: "settings", label: "Settings", run: () => setState({ settingsOpen: true }) },
    { id: "keys", label: "API keys", run: () => setState({ keysOpen: true }) },
    { id: "auth", label: "Auth profiles", run: () => setState({ authOpen: true }) },
    { id: "cap", label: "Captures / diff", run: () => setState({ captureOpen: true }) },
    { id: "add-ssh", label: "New SSH session", run: () => startDeviceEditor(undefined, "ssh") },
    { id: "add-serial", label: "New serial console", run: () => startDeviceEditor(undefined, "serial") },
    { id: "add-api", label: "New API controller", run: () => startDeviceEditor(undefined, "api") },
    { id: "zoom-in", label: "Larger text", run: () => { bumpUiScale(0.1); bumpTermFont(1); } },
    { id: "zoom-out", label: "Smaller text", run: () => { bumpUiScale(-0.1); bumpTermFont(-1); } },
    { id: "zoom-reset", label: "Reset appearance", run: () => resetAppearance() },
    { id: "theme-midnight", label: "Theme: Midnight", run: () => patchAppearance({ theme: "midnight" }) },
    { id: "theme-paper", label: "Theme: Paper", run: () => patchAppearance({ theme: "paper" }) },
    { id: "theme-contrast", label: "Theme: High contrast", run: () => patchAppearance({ theme: "contrast" }) },
    { id: "theme-nord", label: "Theme: Nord", run: () => patchAppearance({ theme: "nord" }) },
    ...devices.map((d) => ({ id: d.id, label: `Connect ${d.name}`, run: () => void openSession(d) })),
  ].filter((i) => i.label.toLowerCase().includes(q.toLowerCase()));
  return (
    <div className="modal-root" onMouseDown={() => setState({ paletteOpen: false })}>
      <div className="palette" onMouseDown={(e) => e.stopPropagation()}>
        <input autoFocus placeholder="Command palette" value={q} onChange={(e) => setQ(e.target.value)} />
        {items.slice(0, 20).map((i) => (
          <button
            key={i.id}
            className="item"
            onClick={() => {
              i.run();
              setState({ paletteOpen: false });
            }}
          >
            {i.label}
          </button>
        ))}
      </div>
    </div>
  );
}

function ImportModal() {
  const [path, setPath] = useState("");
  const [kind, setKind] = useState("ssh_config");
  return (
    <div className="modal-root" onMouseDown={() => setState({ importOpen: false })}>
      <div className="modal" onMouseDown={(e) => e.stopPropagation()}>
        <h2>Import inventory</h2>
        <p>Pass a filesystem path. Passwords in CSV/XML are ignored — map logins to auth profiles after import.</p>
        <label>
          Kind
          <select value={kind} onChange={(e) => setKind(e.target.value)}>
            <option value="ssh_config">OpenSSH config</option>
            <option value="csv">CSV</option>
            <option value="xml">SecureCRT / MTPuTTY XML</option>
          </select>
        </label>
        <label>
          Path
          <input value={path} onChange={(e) => setPath(e.target.value)} placeholder="/home/you/.ssh/config" />
        </label>
        <div className="actions">
          <button className="ghost" onClick={() => setState({ importOpen: false })}>Close</button>
          <button className="primary" onClick={() => void importFile(path, kind)}>Import</button>
        </div>
      </div>
    </div>
  );
}

async function importKeyFile(name: string, file: File) {
  if (file.size > 8192) {
    toast("error", "key file is too large (max 8 KiB)");
    return;
  }
  const text = (await file.text()).trim();
  if (text.length < 8) {
    toast("error", "key file is empty or too short");
    return;
  }
  await saveProviderKey(name, text);
}

function ProviderKeyRow({ name, label }: { name: string; label: string }) {
  const saved = useApp((s) => Boolean(s.providerStatus[name]));
  const [value, setValue] = useState("");
  return (
    <label>
      {label}
      {saved ? <span className="kind-pill"> saved</span> : null}
      <span className="row">
        <input
          type="password"
          autoComplete="off"
          spellCheck={false}
          placeholder={saved ? "••••••••  (paste a new key to replace)" : "paste key — never shown again"}
          value={value}
          onChange={(e) => setValue(e.target.value)}
        />
        <button
          className="primary"
          type="button"
          disabled={value.trim().length < 8}
          onClick={() => {
            void saveProviderKey(name, value).then(() => setValue(""));
          }}
        >
          Save
        </button>
        <label className="ghost" style={{ margin: 0, padding: "8px 12px", cursor: "pointer" }}>
          Import file
          <input
            type="file"
            accept=".txt,.key,.pem,text/plain"
            style={{ display: "none" }}
            onChange={(e) => {
              const file = e.target.files?.[0];
              e.target.value = "";
              if (file) void importKeyFile(name, file);
            }}
          />
        </label>
        {saved && (
          <button className="danger" type="button" onClick={() => void clearProviderKey(name)}>
            Clear
          </button>
        )}
      </span>
    </label>
  );
}

function KeysModal() {
  return (
    <div className="modal-root" onMouseDown={() => setState({ keysOpen: false })}>
      <div className="modal wide" onMouseDown={(e) => e.stopPropagation()}>
        <h2>API keys</h2>
        <p className="hint">
          Paste a key or import a small text file. Keys are written to a 0600 vault on your computer
          (<code>~/.config/late/provider-keys.json</code>), never into chat, never returned to the UI,
          and never visible to the six agent tools. Environment variables still override if set.
        </p>
        <ProviderKeyRow name="cursor" label="Cursor API key" />
        <ProviderKeyRow name="openai" label="OpenAI API key" />
        <ProviderKeyRow name="anthropic" label="Anthropic API key" />
        <ProviderKeyRow name="groq" label="Groq API key" />
        <ProviderKeyRow name="openrouter" label="OpenRouter API key" />
        <ProviderKeyRow name="custom" label="Custom OpenAI-compatible key" />
        <div className="actions">
          <button className="ghost" onClick={() => setState({ keysOpen: false })}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}

function SettingsModal() {
  const settings = useApp((s) => s.settings);
  const uiScale = useApp((s) => s.uiScale);
  const termFontSize = useApp((s) => s.termFontSize);
  const theme = useApp((s) => s.theme);
  const density = useApp((s) => s.density);
  const radius = useApp((s) => s.radius);
  const uiFont = useApp((s) => s.uiFont);
  const layout = useApp((s) => s.layout);
  const [bind, setBind] = useState(settings?.bind ?? "127.0.0.1:7420");
  const [vllm, setVllm] = useState(settings?.vllm_base_url ?? "http://127.0.0.1:8000/v1");
  const [model, setModel] = useState(settings?.vllm_model ?? "local");
  const [ollama, setOllama] = useState(settings?.ollama_base_url ?? "http://127.0.0.1:11434/v1");
  const [ollamaModel, setOllamaModel] = useState(settings?.ollama_model ?? "");
  const [llamaCpp, setLlamaCpp] = useState(settings?.llama_cpp_base_url ?? "http://127.0.0.1:8080/v1");
  const [llamaCppModel, setLlamaCppModel] = useState(settings?.llama_cpp_model ?? "");
  const [defaultBackend, setDefaultBackend] = useState(() => coerceChatBackend(settings?.default_backend));
  const [insecureTls, setInsecureTls] = useState(Boolean(settings?.api_insecure_tls));
  const [cloudChat, setCloudChat] = useState(Boolean(settings?.cloud_chat_enabled));
  return (
    <div className="modal-root" onMouseDown={() => setState({ settingsOpen: false })}>
      <div className="modal wide" onMouseDown={(e) => e.stopPropagation()}>
        <h2>Settings</h2>
        <label>Daemon bind<input value={bind} onChange={(e) => setBind(e.target.value)} /></label>
        <label>
          Default chat backend
          <select
            value={defaultBackend}
            onChange={(e) => {
              const next = coerceChatBackend(e.target.value);
              if (next === "cursor" && !cloudChat) return;
              setDefaultBackend(next);
            }}
          >
            <option value="local">local vLLM</option>
            <option value="llamacpp">llama.cpp</option>
            <option value="ollama">Ollama</option>
            <option value="cursor" disabled={!cloudChat}>
              Cursor SDK{cloudChat ? "" : " — Cloud AI is off"}
            </option>
          </select>
        </label>
        <div className={cloudChat ? "setting-switch on" : "setting-switch"}>
          <label className="setting-switch-row">
            <span className="setting-switch-copy">
              <strong>Cloud AI</strong>
              <span>
                Cursor and remote OpenAI-compatible URLs. Off by default. Hugging Face downloads and Ollama Pull stay available either way.
              </span>
            </span>
            <input
              type="checkbox"
              role="switch"
              aria-checked={cloudChat}
              aria-label="Cloud AI"
              checked={cloudChat}
              onChange={(e) => {
                const on = e.target.checked;
                setCloudChat(on);
                if (!on && defaultBackend === "cursor") setDefaultBackend("local");
              }}
            />
          </label>
          <p className="setting-switch-note">
            {cloudChat
              ? "On. Session text may leave your computer. Save to apply."
              : "Off. Agent chat stays on loopback until this is turned on and saved."}
          </p>
        </div>
        <label>vLLM base URL<input value={vllm} onChange={(e) => setVllm(e.target.value)} /></label>
        <label>Local vLLM model<input value={model} onChange={(e) => setModel(e.target.value)} /></label>
        <p className="hint">
          Default <code>http://127.0.0.1:8000/v1</code>. NVIDIA, AMD, and Intel GPUs all work if the vLLM you run uses them. Late does not bundle a GPU runtime.
        </p>
        <label>llama.cpp base URL<input value={llamaCpp} onChange={(e) => setLlamaCpp(e.target.value)} /></label>
        <label>llama.cpp model (optional)<input value={llamaCppModel} onChange={(e) => setLlamaCppModel(e.target.value)} placeholder="first listed model" /></label>
        <p className="hint">
          llama.cpp is not bundled. Download GGUF from Hugging Face in the Agent pane, then Start if <code>llama-server</code> is on PATH, or run it yourself on <code>127.0.0.1:8080</code>.
          Gated Hub repos need <code>HF_TOKEN</code> (or <code>HUGGING_FACE_HUB_TOKEN</code>) in the environment.
        </p>
        <label>Ollama base URL<input value={ollama} onChange={(e) => setOllama(e.target.value)} /></label>
        <label>Ollama model (optional)<input value={ollamaModel} onChange={(e) => setOllamaModel(e.target.value)} placeholder="first pulled model" /></label>
        <p className="hint">
          Ollama is not bundled. Install from <a href="https://ollama.com" target="_blank" rel="noreferrer">ollama.com</a>
          and start it. Pull library names or Hugging Face ids from the Agent pane.
        </p>
        {defaultBackend === "cursor" && (
          <p className="hint">
            Cursor uses the API key stored under API keys. vLLM Start/Download stay in the Agent pane when Local is selected; llama.cpp and Ollama have their own download/pull controls.
          </p>
        )}
        <label className="check">
          <input type="checkbox" checked={insecureTls} onChange={(e) => setInsecureTls(e.target.checked)} />
          Allow invalid TLS certificates on API sessions (lab gear only)
        </label>
        <label>
          UI scale ({Math.round(uiScale * 100)}%)
          <input
            type="range"
            min={0.85}
            max={2}
            step={0.05}
            value={uiScale}
            onChange={(e) => setAppearance(Number(e.target.value), undefined)}
          />
        </label>
        <label>
          Terminal font ({termFontSize}px)
          <input
            type="range"
            min={12}
            max={36}
            step={1}
            value={termFontSize}
            onChange={(e) => setAppearance(undefined, Number(e.target.value))}
          />
        </label>
        <p className="hint">Ctrl + / − / 0 or Ctrl+mouse wheel also zoom. Default is 115% UI and 18px terminal.</p>
        <div className="appearance-block">
          <h3>Theme</h3>
          <div className="choice-grid">
            {THEMES.map((t) => (
              <button
                key={t.id}
                type="button"
                className={theme === t.id ? "choice on" : "choice"}
                onClick={() => patchAppearance({ theme: t.id })}
              >
                <span className="choice-swatch">
                  <i style={{ background: t.swatch[0] }} />
                  <i style={{ background: t.swatch[1] }} />
                </span>
                <strong>{t.name}</strong>
                <span>{t.hint}</span>
              </button>
            ))}
          </div>
          <h3>Density</h3>
          <div className="choice-grid">
            {DENSITIES.map((d) => (
              <button
                key={d.id}
                type="button"
                className={density === d.id ? "choice on" : "choice"}
                onClick={() => patchAppearance({ density: d.id })}
              >
                <strong>{d.name}</strong>
                <span>{d.hint}</span>
              </button>
            ))}
          </div>
          <h3>Corners</h3>
          <div className="choice-grid">
            {RADII.map((r) => (
              <button
                key={r.id}
                type="button"
                className={radius === r.id ? "choice on" : "choice"}
                onClick={() => patchAppearance({ radius: r.id })}
              >
                <strong>{r.name}</strong>
              </button>
            ))}
          </div>
          <h3>Typeface</h3>
          <div className="choice-grid">
            {FONTS.map((f) => (
              <button
                key={f.id}
                type="button"
                className={uiFont === f.id ? "choice on" : "choice"}
                onClick={() => patchAppearance({ font: f.id, uiFont: f.id })}
              >
                <strong>{f.name}</strong>
                <span>{f.hint}</span>
              </button>
            ))}
          </div>
          <h3>Layout</h3>
          <p className="hint">
            Drag the Sessions, Terminal, or Agent titles to rearrange. Drag the dividers between panes to resize.
            Agent → Top puts chat across the full width (drag the bar under it for height). Agent → Wide grows the side column.
            Inside Agent, Hide the models block or drag the bar under it so the transcript gets the room.
          </p>
          <div className="choice-grid">
            {LAYOUTS.map((l) => (
              <button
                key={l.id}
                type="button"
                className={layout === l.id ? "choice on" : "choice"}
                onClick={() => patchAppearance({ layout: l.id })}
              >
                <strong>{l.name}</strong>
                <span>{l.hint}</span>
              </button>
            ))}
          </div>
          <button type="button" className="ghost" onClick={() => resetAppearance()}>
            Reset look
          </button>
        </div>
        <h3 className="hint" style={{ marginTop: 12 }}>API keys (your computer only)</h3>
        <p className="hint">
          Keys are stored in <code>~/.config/late/provider-keys.json</code> mode 0600, separate from SSH passwords.
          The UI is write-only — saved values are never read back. The agent tools cannot fetch them.
          Env vars still override if set.
        </p>
        <ProviderKeyRow name="cursor" label="Cursor API key" />
        <ProviderKeyRow name="openai" label="OpenAI API key" />
        <ProviderKeyRow name="anthropic" label="Anthropic API key" />
        <ProviderKeyRow name="groq" label="Groq API key" />
        <ProviderKeyRow name="openrouter" label="OpenRouter API key" />
        <ProviderKeyRow name="custom" label="Custom OpenAI-compatible key" />
        <div className="actions">
          <button className="ghost" onClick={() => setState({ settingsOpen: false })}>Close</button>
          <button
            className="primary"
            onClick={() =>
              void saveSettings({
                bind,
                vllm_base_url: vllm,
                vllm_model: model,
                cursor_model: settings?.cursor_model ?? "composer-2.5",
                ollama_base_url: ollama,
                ollama_model: ollamaModel,
                llama_cpp_base_url: llamaCpp,
                llama_cpp_model: llamaCppModel,
                default_backend: defaultBackend,
                scrollback_lines: settings?.scrollback_lines ?? 32000,
                turn_timeout_secs: settings?.turn_timeout_secs ?? 90,
                max_agent_rounds: settings?.max_agent_rounds ?? 50,
                pcap_dir: settings?.pcap_dir,
                log_dir: settings?.log_dir,
                api_insecure_tls: insecureTls,
                cloud_chat_enabled: cloudChat,
              })
            }
          >
            Save
          </button>
        </div>
      </div>
    </div>
  );
}

function CollectionsModal() {
  const collections = useApp((s) => (Array.isArray(s.collections) ? s.collections : []));
  const [name, setName] = useState("lab checks");
  const [cmds, setCmds] = useState("show version\nshow ip route");
  return (
    <div className="modal-root" onMouseDown={() => setState({ collectionsOpen: false })}>
      <div className="modal" onMouseDown={(e) => e.stopPropagation()}>
        <h2>Command collections</h2>
        <p className="hint">
          Named playbooks of CLI commands — health checks, show-tech, post-change verify.
          <strong> Run</strong> sends them into the focused SSH, serial, or local terminal, one line at a time.
          The agent never fires these; you do.
        </p>
        {collections.length === 0 && <p className="hint">No collections yet. Save a named command list below.</p>}
        {collections.map((c) => (
          <div key={c.id} className="file">
            {c.name} ({(c.commands ?? []).length})
            <button className="ghost" onClick={() => void runCollection(c.id)}>Run</button>
            <button className="ghost" onClick={() => void deleteCollection(c.id)}>delete</button>
          </div>
        ))}
        <label>Name<input value={name} onChange={(e) => setName(e.target.value)} /></label>
        <label>Commands<textarea value={cmds} onChange={(e) => setCmds(e.target.value)} /></label>
        <div className="actions">
          <button className="ghost" onClick={() => setState({ collectionsOpen: false })}>Close</button>
          <button
            className="primary"
            onClick={() =>
              void upsertCollection({
                id: newId(),
                name,
                commands: cmds.split("\n").map((s) => s.trim()).filter(Boolean),
              })
            }
          >
            Save collection
          </button>
        </div>
      </div>
    </div>
  );
}

function CaptureModal() {
  const captures = useApp((s) => s.captures);
  const lastDiff = useApp((s) => s.lastDiff);
  const [a, setA] = useState("");
  const [b, setB] = useState("");
  return (
    <div className="modal-root" onMouseDown={() => setState({ captureOpen: false })}>
      <div className="modal" onMouseDown={(e) => e.stopPropagation()}>
        <h2>Captures</h2>
        {captures.map((c) => (
          <div key={c.id} className="file">
            {c.name} · {c.created_at}
          </div>
        ))}
        <label>
          Diff A
          <select value={a} onChange={(e) => setA(e.target.value)}>
            <option value="">select</option>
            {captures.map((c) => (
              <option key={c.id} value={c.id}>{c.name}</option>
            ))}
          </select>
        </label>
        <label>
          Diff B
          <select value={b} onChange={(e) => setB(e.target.value)}>
            <option value="">select</option>
            {captures.map((c) => (
              <option key={c.id} value={c.id}>{c.name}</option>
            ))}
          </select>
        </label>
        <div className="actions">
          <button className="ghost" onClick={() => setState({ captureOpen: false })}>Close</button>
          <button className="primary" disabled={!a || !b} onClick={() => void diffCaptures(a, b)}>Diff</button>
        </div>
        {lastDiff && (
          <pre className="fp" style={{ maxHeight: 240, overflow: "auto" }}>
            {lastDiff.map((l) => `${l.tag} ${l.text}`).join("")}
          </pre>
        )}
      </div>
    </div>
  );
}

function AuthModal() {
  const auth = useApp((s) => s.auth);
  const [name, setName] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [keyPath, setKeyPath] = useState("");
  return (
    <div className="modal-root" onMouseDown={() => setState({ authOpen: false })}>
      <div className="modal" onMouseDown={(e) => e.stopPropagation()}>
        <h2>Auth profiles</h2>
        <p>Passwords are write-only. The UI never reads them back.</p>
        {auth.map((p) => (
          <div key={p.id} className="file">
            {p.name} · {p.username} {p.has_password ? "· password set" : ""} {p.key_path ? `· ${p.key_path}` : ""}
            <button className="ghost" onClick={() => void deleteAuth(p.id)}>delete</button>
          </div>
        ))}
        <label>Name<input value={name} onChange={(e) => setName(e.target.value)} /></label>
        <label>Username<input value={username} onChange={(e) => setUsername(e.target.value)} /></label>
        <label>Password (write-only)<input type="password" value={password} onChange={(e) => setPassword(e.target.value)} /></label>
        <label>Key path<input value={keyPath} onChange={(e) => setKeyPath(e.target.value)} /></label>
        <div className="actions">
          <button className="ghost" onClick={() => setState({ authOpen: false })}>Close</button>
          <button
            className="primary"
            onClick={() => {
              const profile: AuthProfile = {
                id: newId(),
                name,
                username,
                key_path: keyPath || null,
                use_agent: false,
                has_password: Boolean(password),
              };
              void upsertAuth(profile, password || undefined);
            }}
          >
            Save profile
          </button>
        </div>
      </div>
    </div>
  );
}

const SESSION_TYPES: { kind: DeviceKind; title: string; hint: string }[] = [
  { kind: "ssh", title: "SSH", hint: "Router, switch, Linux host — host + port 22" },
  { kind: "serial", title: "Serial", hint: "Console cable — COM / ttyUSB, baud" },
  { kind: "api", title: "API", hint: "Controller REST — base URL only" },
  { kind: "local", title: "Local", hint: "Shell on your computer" },
];

const BAUD_RATES = [9600, 19200, 38400, 57600, 115200];

function pick<T>(d: Device, snake: keyof Device, camel: string): T | undefined {
  const rec = d as unknown as Record<string, unknown>;
  return (rec[snake as string] ?? rec[camel]) as T | undefined;
}

function DeviceModal({ device }: { device: Device }) {
  const [d, setD] = useState(device);
  const [ports, setPorts] = useState<string[]>([]);
  const [more, setMore] = useState(false);
  const auth = useApp((s) => s.auth);
  const folders = useApp((s) => s.inventory.folders);
  const linked = auth.find((p) => p.id === (device.auth_profile_id ?? ""));
  const [username, setUsername] = useState(linked?.username ?? "");
  const [password, setPassword] = useState("");
  const [keyPath, setKeyPath] = useState(linked?.key_path ?? "");

  useEffect(() => {
    if (d.kind !== "serial") return;
    void rpc
      .call<string[]>("serial.list")
      .then((list) => setPorts(Array.isArray(list) ? list : []))
      .catch(() => setPorts(["/dev/ttyUSB0", "/dev/ttyACM0"]));
  }, [d.kind]);

  function patch(p: Partial<Device>) {
    setD((cur) => ({ ...cur, ...p }));
  }

  function setKind(kind: DeviceKind) {
    const next = emptyDevice(kind);
    setD({
      ...next,
      id: d.id,
      name: d.name,
      folder: d.folder,
      tags: d.tags,
      accent: d.accent,
      vendor: kind === "local" ? "linux" : d.vendor,
    });
  }

  const serialPath = pick<string>(d, "serial_path", "serialPath") ?? d.serial_path ?? "";
  const authId = pick<string>(d, "auth_profile_id", "authProfileId") ?? d.auth_profile_id ?? "";
  const apiUrl = pick<string>(d, "api_base_url", "apiBaseUrl") ?? d.api_base_url ?? "";
  const apiCtl = pick<string>(d, "api_controller", "apiController") ?? d.api_controller ?? "";

  return (
    <div className="modal-root" onMouseDown={() => setState({ deviceEditor: null })}>
      <div className="modal wide" onMouseDown={(e) => e.stopPropagation()}>
        <h2>{device.name ? `Edit ${device.name}` : "New session"}</h2>
        <p className="hint">Pick a session type first — only that type’s settings are shown.</p>
        <div className="kind-grid">
          {SESSION_TYPES.map((t) => (
            <button
              key={t.kind}
              type="button"
              className={`kind-card ${d.kind === t.kind ? "active" : ""}`}
              onClick={() => setKind(t.kind)}
            >
              <strong>{t.title}</strong>
              <small>{t.hint}</small>
            </button>
          ))}
        </div>

        <div className="row">
          <label>
            Session name
            <input
              autoFocus
              placeholder={d.kind === "serial" ? "core-sw1 console" : "core-sw1"}
              value={d.name}
              onChange={(e) => patch({ name: e.target.value })}
            />
          </label>
          <label>
            Folder
            <input
              list="late-folders"
              placeholder="Sites/NYC/Core"
              value={d.folder ?? ""}
              onChange={(e) => patch({ folder: e.target.value || null })}
            />
            <datalist id="late-folders">
              {folders.map((f) => (
                <option key={f} value={f} />
              ))}
            </datalist>
          </label>
        </div>
        {d.kind !== "local" && (
          <label>
            Vendor / OS (permit list)
            <select
              value={d.vendor}
              onChange={(e) => patch({ vendor: e.target.value as Device["vendor"] })}
            >
              {VENDORS.map((v) => (
                <option key={v} value={v}>
                  {v === "aos_cx" ? "AOS-CX (Aruba)" : v === "generic" ? "generic (auto-detect)" : v}
                </option>
              ))}
            </select>
          </label>
        )}

        {d.kind === "ssh" && (
          <>
            <div className="row">
              <label>
                Hostname / IP
                <input
                  placeholder="192.0.2.1 or core-sw1.lab"
                  value={d.host ?? ""}
                  onChange={(e) => patch({ host: e.target.value })}
                />
              </label>
              <label>
                SSH port
                <input
                  type="number"
                  value={d.port ?? 22}
                  onChange={(e) => patch({ port: Number(e.target.value) })}
                />
              </label>
            </div>
            <div className="row">
              <label>
                Username
                <input
                  placeholder="admin"
                  autoComplete="off"
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                />
              </label>
              <label>
                Password
                <input
                  type="password"
                  autoComplete="new-password"
                  placeholder={
                    auth.find((p) => p.id === authId)?.has_password
                      ? "saved — leave blank to keep"
                      : "saved with this session"
                  }
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                />
              </label>
            </div>
            <p className="hint">
              Late logs in automatically from this session. The password is stored in a 0600 secret
              file, never in inventory and never on the SSH command line.
            </p>
            <label>
              Private key (optional)
              <input
                placeholder="/home/you/.ssh/id_ed25519"
                value={keyPath}
                onChange={(e) => setKeyPath(e.target.value)}
              />
            </label>
            {auth.length > 0 && (
              <label>
                Or reuse a saved login
                <select
                  value={authId}
                  onChange={(e) => {
                    const id = e.target.value || null;
                    patch({ auth_profile_id: id });
                    const p = auth.find((x) => x.id === id);
                    if (p) {
                      setUsername(p.username);
                      setKeyPath(p.key_path ?? "");
                      setPassword("");
                    }
                  }}
                >
                  <option value="">new login for this session</option>
                  {auth.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.name} ({p.username})
                    </option>
                  ))}
                </select>
              </label>
            )}
            <label>
              Jump host
              <input
                placeholder="optional bastion"
                value={d.jump_host ?? ""}
                onChange={(e) => patch({ jump_host: e.target.value || null })}
              />
            </label>
            <label className="check">
              <input
                type="checkbox"
                checked={d.legacy_ssh}
                onChange={(e) => patch({ legacy_ssh: e.target.checked })}
              />
              Allow older SSH algorithms (legacy gear)
            </label>
          </>
        )}

        {d.kind === "serial" && (
          <>
            <div className="row">
              <label>
                Serial port
                <select
                  value={serialPath}
                  onChange={(e) => patch({ serial_path: e.target.value })}
                >
                  <option value="">Select a port…</option>
                  {ports.map((p) => (
                    <option key={p} value={p}>
                      {p}
                    </option>
                  ))}
                  {serialPath && !ports.includes(serialPath) && (
                    <option value={serialPath}>{serialPath}</option>
                  )}
                </select>
              </label>
              <label>
                Baud
                <select
                  value={String(d.baud ?? 9600)}
                  onChange={(e) => patch({ baud: Number(e.target.value) })}
                >
                  {BAUD_RATES.map((b) => (
                    <option key={b} value={b}>
                      {b}
                    </option>
                  ))}
                </select>
              </label>
            </div>
            <label>
              Or type a device path
              <input
                placeholder="/dev/ttyUSB0"
                value={serialPath}
                onChange={(e) => patch({ serial_path: e.target.value })}
              />
            </label>
            <p className="hint">
              Linux needs your user in the <code>dialout</code> group to open USB serial:
              <code> sudo usermod -aG dialout $USER </code>
              then log out and back in.
            </p>
          </>
        )}

        {d.kind === "api" && (
          <>
            <label>
              Base URL
              <input
                placeholder="https://controller.example:8443"
                value={apiUrl}
                onChange={(e) => patch({ api_base_url: e.target.value })}
              />
            </label>
            <label>
              Controller
              <input
                placeholder="unifi, meraki, generic…"
                value={apiCtl}
                onChange={(e) => patch({ api_controller: e.target.value })}
              />
            </label>
            <label>
              Auth profile
              <select
                value={authId}
                onChange={(e) => patch({ auth_profile_id: e.target.value || null })}
              >
                <option value="">none</option>
                {auth.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
              </select>
            </label>
          </>
        )}

        {d.kind === "local" && (
          <label>
            Shell
            <input value={d.shell ?? "/bin/bash"} onChange={(e) => patch({ shell: e.target.value })} />
          </label>
        )}

        <button type="button" className="linkish" onClick={() => setMore((m) => !m)}>
          {more ? "Hide extra options" : "Vendor, tags, color…"}
        </button>
        {more && (
          <>
            <div className="row">
              <label>
                Vendor / OS
                <select
                  value={d.vendor}
                  onChange={(e) => patch({ vendor: e.target.value as Device["vendor"] })}
                >
                  {VENDORS.map((v) => (
                    <option key={v}>{v}</option>
                  ))}
                </select>
              </label>
              <label>
                Accent
                <input value={d.accent ?? ""} onChange={(e) => patch({ accent: e.target.value })} />
              </label>
            </div>
            <label>
              Tags (comma)
              <input
                value={d.tags.join(",")}
                onChange={(e) =>
                  patch({
                    tags: e.target.value
                      .split(",")
                      .map((s) => s.trim())
                      .filter(Boolean),
                  })
                }
              />
            </label>
          </>
        )}

        <div className="actions">
          <button className="ghost" onClick={() => setState({ deviceEditor: null })}>
            Cancel
          </button>
          <button
            className="primary"
            onClick={() => {
              const name = d.name.trim() || (d.kind === "serial" ? serialPath || "serial" : d.host || "session");
              const body = {
                ...d,
                name,
                folder: normalizeFolderPath(d.folder),
                serial_path: d.kind === "serial" ? serialPath || d.serial_path : null,
                host: d.kind === "ssh" || d.kind === "api" ? d.host : null,
                port: d.kind === "ssh" ? d.port ?? 22 : null,
                baud: d.kind === "serial" ? d.baud ?? 9600 : null,
                api_base_url: d.kind === "api" ? apiUrl : null,
                auth_profile_id: d.kind === "serial" || d.kind === "local" ? null : authId || null,
              };
              if (d.kind === "ssh") {
                void saveDeviceWithLogin(body, {
                  username,
                  password: password || undefined,
                  keyPath: keyPath.trim() || null,
                });
              } else {
                void upsertDevice(body);
              }
            }}
          >
            Save session
          </button>
        </div>
      </div>
    </div>
  );
}
