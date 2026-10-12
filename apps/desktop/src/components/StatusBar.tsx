import { refreshAll, useApp } from "../store";

export function StatusBar() {
  const daemonOk = useApp((s) => s.daemonOk);
  const daemonError = useApp((s) => s.daemonError);
  const sidecarOk = useApp((s) => s.sidecarOk);
  const sessionName = useApp((s) => {
    const pane = s.focusedPaneId ? s.panes[s.focusedPaneId] : undefined;
    return pane?.session?.name ?? "";
  });
  const sessionKind = useApp((s) => {
    const pane = s.focusedPaneId ? s.panes[s.focusedPaneId] : undefined;
    if (!pane?.session) return "";
    return pane.kind === "sftp" ? "scp" : pane.kind;
  });
  const disconnected = useApp((s) => {
    const pane = s.focusedPaneId ? s.panes[s.focusedPaneId] : undefined;
    return Boolean(pane?.session && pane.disconnected);
  });
  const host = useApp((s) => {
    const pane = s.focusedPaneId ? s.panes[s.focusedPaneId] : undefined;
    if (!pane?.deviceId) return "";
    return s.inventory.devices.find((d) => d.id === pane.deviceId)?.host?.trim() ?? "";
  });
  const sessionCount = useApp((s) => s.sessions.length);
  return (
    <footer className="status">
      <span title={daemonOk ? "Daemon on this computer" : (daemonError ?? "Daemon offline")}>
        <i className={`led ${daemonOk ? "ok" : "bad"}`} />
        {daemonOk ? "daemon" : daemonError ?? "daemon offline"}
      </span>
      <span title={sidecarOk ? "Sidecar on this computer" : "Sidecar offline"}>
        <i className={`led ${sidecarOk ? "ok" : "bad"}`} />
        sidecar
      </span>
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
      <span className="status-session">
        {sessionName ? (
          <>
            {sessionName}
            <span className="status-meta">
              {" · "}
              {sessionKind}
              {host ? ` ${host}` : ""}
              {disconnected ? " · disconnected" : ""}
            </span>
          </>
        ) : (
          <span className="status-meta">
            {sessionCount} session{sessionCount === 1 ? "" : "s"}
          </span>
        )}
      </span>
    </footer>
  );
}
