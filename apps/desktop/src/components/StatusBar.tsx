import { refreshAll, setState, useApp } from "../store";

export function StatusBar() {
  const daemonOk = useApp((s) => s.daemonOk);
  const daemonError = useApp((s) => s.daemonError);
  const sidecarOk = useApp((s) => s.sidecarOk);
  const sessions = useApp((s) => s.sessions);
  return (
    <footer className="status">
      <span>
        <i className={`led ${daemonOk ? "ok" : "bad"}`} />
        daemon {daemonOk ? "online" : daemonError ?? "offline"}
      </span>
      <span>
        <i className={`led ${sidecarOk ? "ok" : "bad"}`} />
        sidecar {sidecarOk ? "online" : "offline"}
      </span>
      <span>{sessions.length} sessions</span>
      {!daemonOk && (
        <button
          type="button"
          className="ghost tiny"
          onClick={() => void refreshAll()}
          title="Retry daemon websocket"
        >
          Retry
        </button>
      )}
      <button
        type="button"
        className="ghost tiny"
        style={{ marginLeft: "auto" }}
        onClick={() => setState({ settingsOpen: true })}
        title="Open settings"
      >
        Settings
      </button>
      <span>Ctrl+K palette · Ctrl+, settings</span>
    </footer>
  );
}
