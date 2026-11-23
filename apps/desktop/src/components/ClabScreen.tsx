import { useEffect, useState } from "react";
import { rpc } from "../lib/rpc";
import { boot } from "../store";
import type { Vendor } from "../types";

type NodeRow = {
  long: string;
  short: string;
  kind: string;
  ip: string;
  vendor: Vendor;
};

type Desk = {
  id: string;
  name: string;
  protocol: string;
  server: string;
  target: string;
  tunneled: boolean;
};

function errText(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export function ClabScreen() {
  const [host, setHost] = useState("");
  const [sshUser, setSshUser] = useState("");
  const [deviceUser, setDeviceUser] = useState("");
  const [password, setPassword] = useState("");
  const [savePassword, setSavePassword] = useState(false);
  const [nodes, setNodes] = useState<NodeRow[]>([]);
  const [picked, setPicked] = useState<string[]>([]);
  const [desks, setDesks] = useState<Desk[]>([]);
  const [busy, setBusy] = useState("");
  const [note, setNote] = useState("");

  useEffect(() => {
    boot();
    void (async () => {
      try {
        const cfg = await rpc.call<{ host?: string; sshUser?: string; deviceUser?: string }>("clab.config", {});
        setHost(cfg.host ?? "");
        setSshUser(cfg.sshUser ?? "");
        setDeviceUser(cfg.deviceUser ?? "");
        const listed = await rpc.call<Desk[]>("desktop.list", {});
        setDesks(Array.isArray(listed) ? listed : []);
      } catch (err) {
        setNote(errText(err));
      }
    })();
  }, []);

  function toggle(long: string) {
    setPicked((cur) => (cur.includes(long) ? cur.filter((id) => id !== long) : [...cur, long]));
  }

  async function refresh() {
    setBusy("Looking for running nodes…");
    setNote("");
    try {
      await rpc.call("clab.config", { save: true, host, sshUser, deviceUser });
      const res = await rpc.call<{ nodes?: NodeRow[] }>("clab.discover", { host, sshUser });
      const list = res.nodes ?? [];
      setNodes(list);
      setPicked(list.map((n) => n.long));
      if (!list.length) setNote("No running nodes with a management address.");
    } catch (err) {
      setNote(errText(err));
    } finally {
      setBusy("");
    }
  }

  async function connect() {
    const chosen = nodes.filter((n) => picked.includes(n.long));
    if (!host.trim() || !sshUser.trim() || !deviceUser.trim()) {
      setNote("Lab host, SSH user, and device user are required.");
      return;
    }
    if (!chosen.length) {
      setNote("Select at least one node.");
      return;
    }
    const openWindow = window.lateRuntime?.openWindow;
    if (!openWindow) {
      setNote("Open this from the Late window so each device can get its own workspace.");
      return;
    }
    setBusy("Opening a window for each device…");
    setNote("");
    try {
      await rpc.call("clab.config", { save: true, host, sshUser, deviceUser });
      const jumpHost = `${sshUser.trim()}@${host.trim()}`;
      for (const node of chosen) {
        await openWindow({
          role: "workspace",
          host: node.ip,
          port: 22,
          username: deviceUser.trim(),
          password,
          savePassword: savePassword && password.length > 0,
          jumpHost,
          name: node.short,
          vendor: node.vendor,
        });
      }
    } catch (err) {
      setNote(errText(err));
    } finally {
      setBusy("");
    }
  }

  async function desktop(protocol: "rdp" | "vnc") {
    const node = nodes.filter((n) => picked.includes(n.long));
    if (node.length !== 1) {
      setNote("Select one node for RDP or VNC.");
      return;
    }
    setBusy(protocol === "rdp" ? "Opening RDP…" : "Opening VNC…");
    setNote("");
    try {
      await rpc.call("desktop.open", {
        protocol,
        host: node[0].ip,
        port: protocol === "rdp" ? 3389 : 5900,
        username: deviceUser.trim(),
        password,
        jump: `${sshUser.trim()}@${host.trim()}`,
        name: node[0].short,
      });
      const listed = await rpc.call<Desk[]>("desktop.list", {});
      setDesks(Array.isArray(listed) ? listed : []);
    } catch (err) {
      setNote(errText(err));
    } finally {
      setBusy("");
    }
  }

  async function stopDesk(id: string) {
    try {
      await rpc.call("desktop.close", { id });
      setDesks((cur) => cur.filter((d) => d.id !== id));
    } catch (err) {
      setNote(errText(err));
    }
  }

  return (
    <div className="clab-screen">
      <header className="clab-head">
        <h1>Containerlab</h1>
        <p>
          Connect opens each selected node in its own Late window, saved in the Containerlab folder.
          Chat in those windows keeps your Agent setting, including the orchestrator.
        </p>
      </header>
      <form
        className="clab-form"
        onSubmit={(e) => {
          e.preventDefault();
          void refresh();
        }}
      >
        <label>
          Lab host
          <input value={host} onChange={(e) => setHost(e.target.value)} autoComplete="off" />
        </label>
        <label>
          SSH user
          <input value={sshUser} onChange={(e) => setSshUser(e.target.value)} autoComplete="off" />
        </label>
        <label>
          Device user
          <input value={deviceUser} onChange={(e) => setDeviceUser(e.target.value)} autoComplete="off" />
        </label>
        <label>
          Device password
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            autoComplete="off"
          />
        </label>
        <label className="clab-check">
          <input
            type="checkbox"
            checked={savePassword}
            onChange={(e) => setSavePassword(e.target.checked)}
          />
          Save device password on this computer
        </label>
        <p className="clab-hint">The lab host uses your SSH key or agent. Late does not store that password.</p>
        <div className="clab-actions">
          <button type="submit" className="ghost" disabled={Boolean(busy)}>
            Refresh
          </button>
          <button type="button" className="primary" disabled={Boolean(busy)} onClick={() => void connect()}>
            Connect
          </button>
          <button type="button" className="ghost" disabled={Boolean(busy)} onClick={() => void desktop("rdp")}>
            RDP
          </button>
          <button type="button" className="ghost" disabled={Boolean(busy)} onClick={() => void desktop("vnc")}>
            VNC
          </button>
        </div>
      </form>
      {busy ? <p className="clab-note">{busy}</p> : null}
      {note ? <p className="clab-note">{note}</p> : null}
      <ul className="clab-nodes">
        {nodes.map((n) => (
          <li key={n.long}>
            <label>
              <input type="checkbox" checked={picked.includes(n.long)} onChange={() => toggle(n.long)} />
              <span>{n.short}</span>
              <span className="clab-meta">
                {n.ip} · {n.kind}
              </span>
            </label>
          </li>
        ))}
      </ul>
      {desks.length ? (
        <section className="clab-desks">
          <h2>Desktop sessions</h2>
          <ul>
            {desks.map((d) => (
              <li key={d.id}>
                <span>
                  {d.protocol.toUpperCase()} {d.name} → {d.server}
                </span>
                <button type="button" className="ghost" onClick={() => void stopDesk(d.id)}>
                  Stop
                </button>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
    </div>
  );
}
