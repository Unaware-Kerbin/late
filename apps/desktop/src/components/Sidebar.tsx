import { useMemo, useState } from "react";
import {
  deleteDevice,
  openLocal,
  openPcapPane,
  openSession,
  setState,
  startDeviceEditor,
  useApp,
} from "../store";
import type { Device, SessionKind } from "../types";

export function Sidebar() {
  const inventory = useApp((s) => s.inventory);
  const selected = useApp((s) => s.selectedDeviceId);
  const [q, setQ] = useState("");
  const [newOpen, setNewOpen] = useState(false);

  const grouped = useMemo(() => {
    const devices = inventory.devices.filter((d) => {
      const hay = `${d.name} ${d.host ?? ""} ${d.tags.join(" ")} ${d.folder ?? ""}`.toLowerCase();
      return hay.includes(q.toLowerCase());
    });
    const folders = new Map<string, Device[]>();
    for (const d of devices) {
      const f = d.folder || "Ungrouped";
      folders.set(f, [...(folders.get(f) ?? []), d]);
    }
    return [...folders.entries()].sort(([a], [b]) => a.localeCompare(b));
  }, [inventory.devices, q]);

  function connect(d: Device, kind?: SessionKind) {
    void openSession(d, kind);
  }

  return (
    <aside className="sidebar">
      <div className="side-head">
        Inventory
        <span>
          <span className="new-menu">
            <button className="icon-btn" onClick={() => setNewOpen((v) => !v)} title="New session">
              +
            </button>
            {newOpen && (
              <div className="new-pop">
                <button onClick={() => { setNewOpen(false); startDeviceEditor(undefined, "ssh"); }}>SSH session</button>
                <button onClick={() => { setNewOpen(false); startDeviceEditor(undefined, "serial"); }}>Serial console</button>
                <button onClick={() => { setNewOpen(false); startDeviceEditor(undefined, "api"); }}>API controller</button>
                <button onClick={() => { setNewOpen(false); void openLocal(); }}>Local shell</button>
                <button onClick={() => { setNewOpen(false); openPcapPane(); }}>Packet capture</button>
              </div>
            )}
          </span>
        </span>
      </div>
      <input className="search" placeholder="Filter hosts, tags, folders" value={q} onChange={(e) => setQ(e.target.value)} />
      <div className="tree">
        <button className="device" onClick={() => void openLocal()}>
          <span className="dot" style={{ background: "var(--accent)" }} />
          <div>
            <div className="name">Local PTY</div>
            <div className="meta">host shell · invisible to agent</div>
          </div>
        </button>
        <button className="device" onClick={() => openPcapPane()}>
          <span className="dot" style={{ background: "var(--info)" }} />
          <div>
            <div className="name">Packet capture</div>
            <div className="meta">pcap file, live tcpdump, Edgeshark, SSH capture</div>
          </div>
        </button>
        <button className="device" onClick={() => openPcapPane()}>
          <span className="dot" style={{ background: "#6cb6ff" }} />
          <div>
            <div className="name">Edgeshark</div>
            <div className="meta">container capture UI · http://127.0.0.1:5001</div>
          </div>
        </button>
        {grouped.map(([folder, devices]) => (
          <div key={folder}>
            <div className="folder">{folder}</div>
            {devices.map((d) => (
              <div
                key={d.id}
                className={`device ${selected === d.id ? "selected" : ""}`}
                onClick={() => setState({ selectedDeviceId: d.id })}
                onDoubleClick={() => connect(d)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") connect(d);
                }}
                tabIndex={0}
              >
                <span className="dot" style={{ background: d.accent || "var(--accent)" }} />
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div className="name">{d.name}</div>
                  <div className="meta">
                    <span className="kind-pill">{d.kind}</span>
                    {d.kind === "ssh" && d.host
                      ? `${d.host}${d.port && d.port !== 22 ? `:${d.port}` : ""}`
                      : d.kind === "serial"
                        ? `${d.serial_path ?? "no port"} @ ${d.baud ?? 9600}`
                        : d.kind === "api"
                          ? (d.api_base_url ?? "no URL")
                          : d.vendor}
                  </div>
                  {(d.tags ?? []).length > 0 && (
                    <div className="tags">
                      {(d.tags ?? []).map((t) => (
                        <span key={t} className="tag">
                          {t}
                        </span>
                      ))}
                    </div>
                  )}
                </div>
                <button className="ghost" onClick={(e) => { e.stopPropagation(); startDeviceEditor(d); }}>
                  edit
                </button>
              </div>
            ))}
          </div>
        ))}
      </div>
      <div className="side-actions">
        <button className="ghost" onClick={() => setState({ keysOpen: true })}>
          API keys
        </button>
        <button className="ghost" onClick={() => setState({ authOpen: true })}>
          Auth
        </button>
        <button className="ghost" onClick={() => setState({ importOpen: true })}>
          Import
        </button>
        <button className="ghost" onClick={() => setState({ collectionsOpen: true })}>
          Collections
        </button>
        <button className="ghost" onClick={() => setState({ settingsOpen: true })}>
          Settings
        </button>
      </div>
      {selected && (
        <div className="row" style={{ padding: "0 8px 8px" }}>
          <button className="primary" onClick={() => {
            const d = inventory.devices.find((x) => x.id === selected);
            if (d) connect(d);
          }}>
            Connect
          </button>
          {inventory.devices.find((x) => x.id === selected)?.kind === "ssh" && (
            <button className="ghost" onClick={() => {
              const d = inventory.devices.find((x) => x.id === selected);
              if (d) connect(d, "sftp");
            }}>
              SFTP
            </button>
          )}
          <button className="danger" onClick={() => void deleteDevice(selected)}>
            Delete
          </button>
        </div>
      )}
    </aside>
  );
}
