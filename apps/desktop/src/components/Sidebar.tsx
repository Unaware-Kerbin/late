import { useEffect, useMemo, useRef, useState, type DragEvent, type MouseEvent, type ReactNode } from "react";
import {
  deleteDevice,
  deleteFolder,
  moveDeviceToFolder,
  openLocal,
  openPcapPane,
  openSession,
  renameFolder,
  setState,
  startDeviceEditor,
  toast,
  upsertFolder,
  useApp,
} from "../store";
import { folderPathIsUnder, normalizeFolderPath, type Device, type SessionKind } from "../types";

type FolderNode = {
  name: string;
  path: string;
  children: FolderNode[];
  devices: Device[];
};

function normalizeFolder(raw: string | null | undefined): string {
  return normalizeFolderPath(raw) ?? "";
}

function loadCollapsed(): Set<string> {
  try {
    const raw = JSON.parse(localStorage.getItem("late.folders.collapsed") || "[]") as unknown;
    return new Set(Array.isArray(raw) ? raw.map(String) : []);
  } catch {
    return new Set();
  }
}

function saveCollapsed(set: Set<string>) {
  try {
    localStorage.setItem("late.folders.collapsed", JSON.stringify([...set]));
  } catch {
    /* ignore */
  }
}

function buildTree(folderList: string[], devices: Device[]): FolderNode {
  const root: FolderNode = { name: "", path: "", children: [], devices: [] };
  const index = new Map<string, FolderNode>([["", root]]);

  const ensure = (path: string): FolderNode => {
    const existing = index.get(path);
    if (existing) return existing;
    const parentPath = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
    const parent = ensure(parentPath);
    const node: FolderNode = {
      name: path.slice(parentPath.length ? parentPath.length + 1 : 0),
      path,
      children: [],
      devices: [],
    };
    parent.children.push(node);
    index.set(path, node);
    return node;
  };

  for (const f of folderList) {
    const path = normalizeFolder(f);
    if (path) ensure(path);
  }
  for (const d of devices) {
    const path = normalizeFolder(d.folder);
    if (!path) root.devices.push(d);
    else ensure(path).devices.push(d);
  }
  const sortNode = (n: FolderNode) => {
    n.children.sort((a, b) => a.name.localeCompare(b.name));
    n.devices.sort((a, b) => a.name.localeCompare(b.name));
    n.children.forEach(sortNode);
  };
  sortNode(root);
  return root;
}

function countDevices(n: FolderNode): number {
  return n.devices.length + n.children.reduce((sum, c) => sum + countDevices(c), 0);
}

function ancestorPaths(path: string): string[] {
  const parts = path.split("/").filter(Boolean);
  const out: string[] = [];
  let acc = "";
  for (const p of parts) {
    acc = acc ? `${acc}/${p}` : p;
    out.push(acc);
  }
  return out;
}

function clampMenuPos(x: number, y: number) {
  const menuW = 176;
  const menuH = 168;
  return {
    x: Math.max(8, Math.min(x, window.innerWidth - menuW - 8)),
    y: Math.max(8, Math.min(y, window.innerHeight - menuH - 8)),
  };
}

export function Sidebar() {
  const inventory = useApp((s) => s.inventory);
  const selected = useApp((s) => s.selectedDeviceId);
  const [q, setQ] = useState("");
  const [newOpen, setNewOpen] = useState(false);
  const [collapsed, setCollapsed] = useState<Set<string>>(loadCollapsed);
  const [menu, setMenu] = useState<{ path: string; x: number; y: number } | null>(null);
  const [dragOver, setDragOver] = useState<string | null>(null);
  const [activeFolder, setActiveFolder] = useState<string | null>(null);
  const [folderDlg, setFolderDlg] = useState<
    { mode: "create"; parent: string } | { mode: "rename"; path: string } | { mode: "delete"; path: string } | null
  >(null);
  const [folderName, setFolderName] = useState("");
  const folderInput = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (folderDlg?.mode === "create" || folderDlg?.mode === "rename") {
      folderInput.current?.focus();
      folderInput.current?.select();
    }
  }, [folderDlg]);

  useEffect(() => {
    if (!menu && !newOpen) return;
    const close = (e: Event) => {
      const t = e.target as HTMLElement | null;
      if (t?.closest(".folder-menu, .folder-dlg, .new-menu")) return;
      setMenu(null);
      setNewOpen(false);
    };
    const id = window.setTimeout(() => document.addEventListener("mousedown", close), 0);
    return () => {
      window.clearTimeout(id);
      document.removeEventListener("mousedown", close);
    };
  }, [menu, newOpen]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (folderDlg) {
        e.preventDefault();
        setFolderDlg(null);
        return;
      }
      if (menu) {
        e.preventDefault();
        setMenu(null);
        return;
      }
      if (newOpen) {
        e.preventDefault();
        setNewOpen(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [folderDlg, menu, newOpen]);

  const tree = useMemo(() => {
    const query = q.trim().toLowerCase();
    const devices = inventory.devices.filter((d) => {
      if (!query) return true;
      const hay = `${d.name} ${d.host ?? ""} ${d.tags.join(" ")} ${d.folder ?? ""}`.toLowerCase();
      return hay.includes(query);
    });
    const extra = query
      ? [
          ...devices.map((d) => normalizeFolder(d.folder)).filter(Boolean),
          ...inventory.folders.filter((f) => f.toLowerCase().includes(query)),
        ]
      : inventory.folders;
    return buildTree(extra, devices);
  }, [inventory.devices, inventory.folders, q]);

  function toggle(path: string) {
    setCollapsed((cur) => {
      const next = new Set(cur);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      saveCollapsed(next);
      return next;
    });
  }

  function expandPaths(paths: string[]) {
    setCollapsed((cur) => {
      const next = new Set(cur);
      let changed = false;
      for (const p of paths) {
        if (next.delete(p)) changed = true;
      }
      if (!changed) return cur;
      saveCollapsed(next);
      return next;
    });
  }

  function selectFolder(path: string) {
    setActiveFolder(path);
    setCollapsed((cur) => {
      if (!cur.has(path)) return cur;
      const next = new Set(cur);
      next.delete(path);
      saveCollapsed(next);
      return next;
    });
  }

  function connect(d: Device, kind?: SessionKind) {
    void openSession(d, kind);
  }

  function askNewFolder(parent: string) {
    setMenu(null);
    setNewOpen(false);
    setFolderName("");
    setFolderDlg({ mode: "create", parent });
  }

  function askRename(path: string) {
    setMenu(null);
    const leaf = path.includes("/") ? path.slice(path.lastIndexOf("/") + 1) : path;
    setFolderName(leaf);
    setFolderDlg({ mode: "rename", path });
  }

  function confirmDelete(path: string) {
    setMenu(null);
    setFolderDlg({ mode: "delete", path });
  }

  async function submitFolderDlg() {
    if (!folderDlg) return;
    if (folderDlg.mode === "delete") {
      const gone = folderDlg.path;
      if (!(await deleteFolder(gone))) return;
      setActiveFolder((cur) => (cur != null && folderPathIsUnder(cur, gone) ? null : cur));
      setFolderDlg(null);
      return;
    }
    const name = normalizeFolder(folderName);
    if (!name) {
      toast("error", "Folder name is empty");
      return;
    }
    if (folderDlg.mode === "create") {
      const path = normalizeFolder(folderDlg.parent ? `${folderDlg.parent}/${name}` : name);
      if (!path) {
        toast("error", "Folder name is empty");
        return;
      }
      if (!(await upsertFolder(path))) return;
      expandPaths(ancestorPaths(path));
      setActiveFolder(path);
    } else {
      const parent = folderDlg.path.includes("/")
        ? folderDlg.path.slice(0, folderDlg.path.lastIndexOf("/"))
        : "";
      const to = normalizeFolder(parent ? `${parent}/${name}` : name);
      if (!to) {
        toast("error", "Folder name is empty");
        return;
      }
      if (to === folderDlg.path) {
        setFolderDlg(null);
        return;
      }
      const from = folderDlg.path;
      if (!(await renameFolder(from, to))) return;
      setActiveFolder((cur) => {
        if (cur == null || cur === "") return cur;
        if (cur === from) return to;
        if (folderPathIsUnder(cur, from)) return `${to}${cur.slice(from.length)}`;
        return cur;
      });
      expandPaths(ancestorPaths(to));
    }
    setFolderDlg(null);
  }

  function onDropFolder(path: string | null, e: DragEvent) {
    e.preventDefault();
    e.stopPropagation();
    setDragOver(null);
    const id = e.dataTransfer.getData("text/late-device");
    if (id) void moveDeviceToFolder(id, path);
  }

  function openFolderMenu(path: string, e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    const pos = clampMenuPos(e.clientX, e.clientY);
    setMenu({ path, x: pos.x, y: pos.y });
  }

  function renderDevice(d: Device, depth: number) {
    return (
      <div
        key={d.id}
        className={`device ${selected === d.id ? "selected" : ""}`}
        style={{ paddingLeft: 10 + depth * 12 }}
        draggable
        onDragStart={(e) => {
          e.dataTransfer.setData("text/late-device", d.id);
          e.dataTransfer.effectAllowed = "move";
        }}
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
        <button
          className="ghost"
          onClick={(e) => {
            e.stopPropagation();
            startDeviceEditor(d);
          }}
        >
          edit
        </button>
      </div>
    );
  }

  function renderFolder(node: FolderNode, depth: number): ReactNode {
    if (!node.path && !node.children.length) {
      return node.devices.map((d) => renderDevice(d, depth));
    }
    if (!node.path) {
      return (
        <>
          {node.children.map((c) => renderFolder(c, depth))}
          {node.devices.length > 0 && (
            <div>
              <div
                className={`folder-row ${dragOver === "" ? "drop" : ""} ${activeFolder === "" ? "active" : ""}`}
                style={{ paddingLeft: 8 + depth * 12 }}
                onClick={() => selectFolder("")}
                onDragOver={(e) => {
                  e.preventDefault();
                  setDragOver("");
                }}
                onDragLeave={(e) => {
                  if (e.currentTarget.contains(e.relatedTarget as Node)) return;
                  setDragOver((cur) => (cur === "" ? null : cur));
                }}
                onDrop={(e) => onDropFolder(null, e)}
              >
                <button
                  type="button"
                  className="chev"
                  aria-label={collapsed.has("") ? "Expand Ungrouped" : "Collapse Ungrouped"}
                  onClick={(e) => {
                    e.stopPropagation();
                    toggle("");
                  }}
                >
                  {collapsed.has("") ? "▸" : "▾"}
                </button>
                <button type="button" className="folder-main" onClick={() => selectFolder("")}>
                  <span className="folder-name">Ungrouped</span>
                  <span className="folder-count">{node.devices.length}</span>
                </button>
              </div>
              {!collapsed.has("") && node.devices.map((d) => renderDevice(d, depth + 1))}
            </div>
          )}
        </>
      );
    }
    const open = q.trim() ? true : !collapsed.has(node.path);
    const n = countDevices(node);
    return (
      <div key={node.path}>
        <div
          className={`folder-row ${dragOver === node.path ? "drop" : ""} ${activeFolder === node.path ? "active" : ""}`}
          style={{ paddingLeft: 8 + depth * 12 }}
          onClick={() => selectFolder(node.path)}
          onContextMenu={(e) => openFolderMenu(node.path, e)}
          onDragOver={(e) => {
            e.preventDefault();
            setDragOver(node.path);
          }}
          onDragLeave={(e) => {
            if (e.currentTarget.contains(e.relatedTarget as Node)) return;
            setDragOver((cur) => (cur === node.path ? null : cur));
          }}
          onDrop={(e) => onDropFolder(node.path, e)}
        >
          <button
            type="button"
            className="chev"
            aria-label={open ? `Collapse ${node.name}` : `Expand ${node.name}`}
            onClick={(e) => {
              e.stopPropagation();
              toggle(node.path);
            }}
          >
            {open ? "▾" : "▸"}
          </button>
          <button type="button" className="folder-main" onClick={() => selectFolder(node.path)}>
            <span className="folder-name">{node.name}</span>
            <span className="folder-count">{n}</span>
          </button>
        </div>
        {open && (
          <>
            {node.children.map((c) => renderFolder(c, depth + 1))}
            {node.devices.map((d) => renderDevice(d, depth + 1))}
          </>
        )}
      </div>
    );
  }

  return (
    <aside
      className="sidebar"
      onClick={(e) => {
        if ((e.target as HTMLElement).closest(".folder-dlg, .folder-menu, .new-pop")) return;
        setMenu(null);
      }}
    >
      <div className="side-head">
        Inventory
        <span className="side-head-actions">
          <button
            type="button"
            className="icon-btn"
            title="New folder"
            onClick={(e) => {
              e.stopPropagation();
              askNewFolder(activeFolder || "");
            }}
          >
            Folder
          </button>
          <span className="new-menu">
            <button
              type="button"
              className="icon-btn"
              onClick={(e) => {
                e.stopPropagation();
                setNewOpen((v) => !v);
              }}
              title="New session"
            >
              +
            </button>
            {newOpen && (
              <div
                className="new-pop"
                onClick={(e) => e.stopPropagation()}
                onMouseDown={(e) => e.stopPropagation()}
              >
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    askNewFolder(activeFolder || "");
                  }}
                >
                  New folder
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setNewOpen(false);
                    startDeviceEditor(undefined, "ssh", activeFolder);
                  }}
                >
                  SSH session
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setNewOpen(false);
                    startDeviceEditor(undefined, "serial", activeFolder);
                  }}
                >
                  Serial console
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setNewOpen(false);
                    startDeviceEditor(undefined, "api", activeFolder);
                  }}
                >
                  API controller
                </button>
                <button
                  onClick={() => {
                    setNewOpen(false);
                    void openLocal();
                  }}
                >
                  Local shell
                </button>
                <button
                  onClick={() => {
                    setNewOpen(false);
                    openPcapPane();
                  }}
                >
                  Packet capture
                </button>
              </div>
            )}
          </span>
        </span>
      </div>
      <input
        className="search"
        placeholder="Filter hosts, tags, folders"
        value={q}
        onChange={(e) => setQ(e.target.value)}
      />
      {folderDlg && (
        <form
          className="folder-dlg"
          onClick={(e) => e.stopPropagation()}
          onMouseDown={(e) => e.stopPropagation()}
          onSubmit={(e) => {
            e.preventDefault();
            submitFolderDlg();
          }}
        >
          {folderDlg.mode === "delete" ? (
            <p>
              Delete folder <strong>{folderDlg.path}</strong>? Devices move to the parent group.
            </p>
          ) : (
            <label>
              {folderDlg.mode === "rename"
                ? "Rename folder"
                : folderDlg.parent
                  ? `New folder under ${folderDlg.parent}`
                  : "New folder"}
              <input
                ref={folderInput}
                value={folderName}
                onChange={(e) => setFolderName(e.target.value)}
                placeholder={folderDlg.mode === "create" && !folderDlg.parent ? "NYC/Core" : "Core"}
              />
            </label>
          )}
          <div className="row">
            <button type="button" className="ghost" onClick={() => setFolderDlg(null)}>
              Cancel
            </button>
            <button
              type="submit"
              className={folderDlg.mode === "delete" ? "danger" : "primary"}
              disabled={folderDlg.mode !== "delete" && !normalizeFolder(folderName)}
            >
              {folderDlg.mode === "delete" ? "Delete" : folderDlg.mode === "rename" ? "Rename" : "Create"}
            </button>
          </div>
        </form>
      )}
      <div
        className="tree"
        onDragOver={(e) => {
          e.preventDefault();
          e.dataTransfer.dropEffect = "move";
        }}
        onDrop={(e) => {
          if ((e.target as HTMLElement).closest(".folder-row, .device")) return;
          onDropFolder(null, e);
        }}
      >
        <button type="button" className="device" onClick={() => void openLocal()}>
          <span className="dot" style={{ background: "var(--accent)" }} />
          <div>
            <div className="name">Local PTY</div>
            <div className="meta">host shell · invisible to agent</div>
          </div>
        </button>
        <button type="button" className="device" onClick={() => openPcapPane()}>
          <span className="dot" style={{ background: "var(--info)" }} />
          <div>
            <div className="name">Packet capture</div>
            <div className="meta">pcap file, live tcpdump, Edgeshark, SSH capture</div>
          </div>
        </button>
        <button type="button" className="device" onClick={() => openPcapPane()}>
          <span className="dot" style={{ background: "#6cb6ff" }} />
          <div>
            <div className="name">Edgeshark</div>
            <div className="meta">container capture UI · http://127.0.0.1:5001</div>
          </div>
        </button>
        {!q.trim() && inventory.folders.length === 0 && inventory.devices.length === 0 && (
          <button
            type="button"
            className="folder-row"
            onClick={(e) => {
              e.stopPropagation();
              askNewFolder("");
            }}
          >
            <span className="folder-name">Create a site folder…</span>
          </button>
        )}
        {renderFolder(tree, 0)}
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
          <button
            className="primary"
            onClick={() => {
              const d = inventory.devices.find((x) => x.id === selected);
              if (d) connect(d);
            }}
          >
            Connect
          </button>
          {inventory.devices.find((x) => x.id === selected)?.kind === "ssh" && (
            <button
              className="ghost"
              onClick={() => {
                const d = inventory.devices.find((x) => x.id === selected);
                if (d) connect(d, "sftp");
              }}
            >
              SFTP
            </button>
          )}
          <button className="danger" onClick={() => void deleteDevice(selected)}>
            Delete
          </button>
        </div>
      )}
      {menu && (
        <div
          className="folder-menu"
          style={{ left: menu.x, top: menu.y }}
          onClick={(e) => e.stopPropagation()}
          onMouseDown={(e) => e.stopPropagation()}
        >
          <button
            type="button"
            onClick={() => {
              setMenu(null);
              askNewFolder(menu.path);
            }}
          >
            New subfolder
          </button>
          <button
            type="button"
            onClick={() => {
              setMenu(null);
              setActiveFolder(menu.path);
              startDeviceEditor(undefined, "ssh", menu.path);
            }}
          >
            New SSH session
          </button>
          <button
            type="button"
            onClick={() => {
              setMenu(null);
              askRename(menu.path);
            }}
          >
            Rename
          </button>
          <button type="button" onClick={() => confirmDelete(menu.path)}>
            Delete folder
          </button>
        </div>
      )}
    </aside>
  );
}
