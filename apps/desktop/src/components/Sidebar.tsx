import { useEffect, useMemo, useRef, useState, type DragEvent, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent, type ReactNode } from "react";
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

type CtxMenu =
  | { kind: "folder"; path: string; x: number; y: number }
  | { kind: "device"; id: string; x: number; y: number };

type VisRow =
  | { key: string; type: "folder"; path: string; name: string; depth: number; count: number }
  | { key: string; type: "device"; device: Device; depth: number }
  | { key: string; type: "tools"; depth: number }
  | { key: string; type: "tool"; id: "pty" | "pcap" | "edgeshark"; depth: number };

const ROOT_COLLAPSE = "__sessions__";
const TOOLS_COLLAPSE = "__tools__";

function normalizeFolder(raw: string | null | undefined): string {
  return normalizeFolderPath(raw) ?? "";
}

function loadCollapsed(): Set<string> {
  try {
    const raw = JSON.parse(localStorage.getItem("late.folders.collapsed") || "[]") as unknown;
    const next = new Set(Array.isArray(raw) ? raw.map(String) : []);
    next.delete("");
    return next;
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

function collectFolderPaths(n: FolderNode, into: string[] = []): string[] {
  for (const c of n.children) {
    into.push(c.path);
    collectFolderPaths(c, into);
  }
  return into;
}

function clampMenuPos(x: number, y: number, h = 176) {
  const menuW = 188;
  return {
    x: Math.max(8, Math.min(x, window.innerWidth - menuW - 8)),
    y: Math.max(8, Math.min(y, window.innerHeight - h - 8)),
  };
}

function deviceHost(d: Device): string {
  if (d.kind === "ssh" && d.host) return d.port && d.port !== 22 ? `${d.host}:${d.port}` : d.host;
  if (d.kind === "serial") return d.serial_path ?? "";
  if (d.kind === "api") return d.api_base_url ?? "";
  return "";
}

function kindLabel(kind: Device["kind"]): string {
  if (kind === "ssh") return "SSH";
  if (kind === "serial") return "Serial";
  if (kind === "api") return "API";
  return kind;
}

function Icon({ children }: { children: ReactNode }) {
  return (
    <svg className="sess-ico" viewBox="0 0 16 16" aria-hidden>
      {children}
    </svg>
  );
}

function FolderIco() {
  return (
    <Icon>
      <path fill="currentColor" d="M1.5 3.5h4.2l.8 1.5H14.5v8H1.5z" opacity="0.92" />
    </Icon>
  );
}

function HostIco() {
  return (
    <Icon>
      <rect x="2" y="3" width="12" height="8" rx="1.2" fill="none" stroke="currentColor" strokeWidth="1.3" />
      <path d="M6 13h4M8 11v2" stroke="currentColor" strokeWidth="1.3" />
    </Icon>
  );
}

function SerialIco() {
  return (
    <Icon>
      <path d="M3 8h10M5 5v6M11 5v6" fill="none" stroke="currentColor" strokeWidth="1.3" />
    </Icon>
  );
}

function ApiIco() {
  return (
    <Icon>
      <circle cx="8" cy="8" r="2" fill="currentColor" />
      <path d="M8 3v2M8 11v2M3 8h2M11 8h2" stroke="currentColor" strokeWidth="1.3" />
    </Icon>
  );
}

function DeviceIco({ kind }: { kind: Device["kind"] }) {
  if (kind === "serial") return <SerialIco />;
  if (kind === "api") return <ApiIco />;
  return <HostIco />;
}

export function Sidebar() {
  const inventory = useApp((s) => s.inventory);
  const selected = useApp((s) => s.selectedDeviceId);
  const [q, setQ] = useState("");
  const [newOpen, setNewOpen] = useState(false);
  const [collapsed, setCollapsed] = useState<Set<string>>(loadCollapsed);
  const [menu, setMenu] = useState<CtxMenu | null>(null);
  const [dragOver, setDragOver] = useState<string | null>(null);
  const [activeFolder, setActiveFolder] = useState("");
  const [folderDlg, setFolderDlg] = useState<
    { mode: "create"; parent: string } | { mode: "rename"; path: string } | { mode: "delete"; path: string } | null
  >(null);
  const [folderName, setFolderName] = useState("");
  const folderInput = useRef<HTMLInputElement>(null);
  const filterRef = useRef<HTMLInputElement>(null);

  const filtering = Boolean(q.trim());
  const toolsOpen = filtering || !collapsed.has(TOOLS_COLLAPSE);

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
      if (e.altKey && (e.key === "i" || e.key === "I")) {
        e.preventDefault();
        filterRef.current?.focus();
        filterRef.current?.select();
        return;
      }
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
      const hay = `${d.name} ${d.host ?? ""} ${d.tags.join(" ")} ${d.folder ?? ""} ${d.kind}`.toLowerCase();
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

  function folderOpen(path: string) {
    if (filtering) return true;
    if (path === "") return !collapsed.has(ROOT_COLLAPSE);
    return !collapsed.has(path);
  }

  const rows = useMemo(() => {
    const isOpen = (path: string) => {
      if (filtering) return true;
      if (path === "") return !collapsed.has(ROOT_COLLAPSE);
      return !collapsed.has(path);
    };
    const out: VisRow[] = [
      { key: "sessions", type: "folder", path: "", name: "Sessions", depth: 0, count: countDevices(tree) },
    ];
    const walk = (node: FolderNode, depth: number) => {
      if (!isOpen(node.path)) return;
      for (const c of node.children) {
        out.push({ key: `f:${c.path}`, type: "folder", path: c.path, name: c.name, depth, count: countDevices(c) });
        walk(c, depth + 1);
      }
      for (const d of node.devices) {
        out.push({ key: `d:${d.id}`, type: "device", device: d, depth });
      }
    };
    walk(tree, 1);
    out.push({ key: "tools", type: "tools", depth: 0 });
    if (toolsOpen) {
      out.push({ key: "t:pty", type: "tool", id: "pty", depth: 1 });
      out.push({ key: "t:pcap", type: "tool", id: "pcap", depth: 1 });
      out.push({ key: "t:edgeshark", type: "tool", id: "edgeshark", depth: 1 });
    }
    return out;
  }, [tree, collapsed, filtering, toolsOpen]);

  const selectedDevice = inventory.devices.find((d) => d.id === selected) ?? null;

  function toggle(path: string) {
    const key = path === "" ? ROOT_COLLAPSE : path;
    setCollapsed((cur) => {
      const next = new Set(cur);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      saveCollapsed(next);
      return next;
    });
  }

  function expandPaths(paths: string[]) {
    setCollapsed((cur) => {
      const next = new Set(cur);
      let changed = false;
      for (const p of paths) {
        if (next.delete(p === "" ? ROOT_COLLAPSE : p)) changed = true;
      }
      if (!changed) return cur;
      saveCollapsed(next);
      return next;
    });
  }

  function collapseAll() {
    const next = new Set<string>([...collectFolderPaths(tree), TOOLS_COLLAPSE]);
    saveCollapsed(next);
    setCollapsed(next);
  }

  function expandAll() {
    saveCollapsed(new Set());
    setCollapsed(new Set());
  }

  function selectFolder(path: string, expand = true) {
    setActiveFolder(path);
    setState({ selectedDeviceId: null });
    if (!expand) return;
    setCollapsed((cur) => {
      const key = path === "" ? ROOT_COLLAPSE : path;
      if (!cur.has(key)) return cur;
      const next = new Set(cur);
      next.delete(key);
      saveCollapsed(next);
      return next;
    });
  }

  function selectDevice(d: Device) {
    setState({ selectedDeviceId: d.id });
    setActiveFolder(normalizeFolder(d.folder));
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
    if (!path) return;
    setMenu(null);
    const leaf = path.includes("/") ? path.slice(path.lastIndexOf("/") + 1) : path;
    setFolderName(leaf);
    setFolderDlg({ mode: "rename", path });
  }

  function confirmDelete(path: string) {
    if (!path) return;
    setMenu(null);
    setFolderDlg({ mode: "delete", path });
  }

  async function submitFolderDlg() {
    if (!folderDlg) return;
    if (folderDlg.mode === "delete") {
      const gone = folderDlg.path;
      if (!(await deleteFolder(gone))) return;
      setActiveFolder((cur) => (folderPathIsUnder(cur, gone) ? "" : cur));
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

  function openMenu(next: CtxMenu, e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    const pos = clampMenuPos(e.clientX, e.clientY, next.kind === "device" ? 200 : 176);
    setMenu({ ...next, x: pos.x, y: pos.y });
  }

  function currentIndex() {
    if (selected) {
      const i = rows.findIndex((r) => r.type === "device" && r.device.id === selected);
      if (i >= 0) return i;
    }
    return rows.findIndex((r) => r.type === "folder" && r.path === activeFolder);
  }

  function activateRow(row: VisRow) {
    if (row.type === "folder") selectFolder(row.path, false);
    else if (row.type === "device") selectDevice(row.device);
    else if (row.type === "tools") setState({ selectedDeviceId: null });
  }

  function onTreeKey(e: ReactKeyboardEvent<HTMLElement>) {
    const i = currentIndex();
    if (e.key === "ArrowDown") {
      e.preventDefault();
      const next = rows[Math.min(rows.length - 1, Math.max(0, i) + 1)];
      if (next) activateRow(next);
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      const next = rows[Math.max(0, (i < 0 ? 0 : i) - 1)];
      if (next) activateRow(next);
      return;
    }
    if (e.key === "ArrowRight") {
      e.preventDefault();
      const row = i >= 0 ? rows[i] : null;
      if (row?.type === "folder" && !folderOpen(row.path)) toggle(row.path);
      if (row?.type === "tools" && !toolsOpen) toggle(TOOLS_COLLAPSE);
      return;
    }
    if (e.key === "ArrowLeft") {
      e.preventDefault();
      const row = i >= 0 ? rows[i] : null;
      if (row?.type === "folder" && folderOpen(row.path) && (row.path !== "" || tree.children.length + tree.devices.length > 0)) {
        toggle(row.path);
        return;
      }
      if (row?.type === "device") {
        selectFolder(normalizeFolder(row.device.folder), false);
      }
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      if (selectedDevice) connect(selectedDevice);
      else if (activeFolder) toggle(activeFolder);
      return;
    }
    if (e.key === "F2" && activeFolder && !selectedDevice) {
      e.preventDefault();
      askRename(activeFolder);
      return;
    }
    if (e.key === "Delete") {
      if (selectedDevice) {
        e.preventDefault();
        void deleteDevice(selectedDevice.id);
      } else if (activeFolder) {
        e.preventDefault();
        confirmDelete(activeFolder);
      }
    }
  }

  function renderRow(row: VisRow) {
    const pad = { paddingLeft: 6 + row.depth * 14 };
    if (row.type === "folder") {
      const open = folderOpen(row.path);
      const dropKey = row.path;
      const active = !selected && activeFolder === row.path;
      return (
        <div
          key={row.key}
          className={`tree-row folder ${dragOver === dropKey ? "drop" : ""} ${active ? "active" : ""}`}
          style={pad}
          role="treeitem"
          tabIndex={-1}
          aria-expanded={open}
          aria-selected={active}
          onClick={(e) => {
            e.currentTarget.focus();
            selectFolder(row.path, false);
          }}
          onDoubleClick={() => toggle(row.path)}
          onContextMenu={(e) => {
            if (!row.path) return;
            openMenu({ kind: "folder", path: row.path, x: 0, y: 0 }, e);
          }}
          onDragOver={(e) => {
            e.preventDefault();
            setDragOver(dropKey);
          }}
          onDragLeave={(e) => {
            if (e.currentTarget.contains(e.relatedTarget as Node)) return;
            setDragOver((cur) => (cur === dropKey ? null : cur));
          }}
          onDrop={(e) => onDropFolder(row.path || null, e)}
        >
          <button
            type="button"
            className={`chev ${open ? "open" : ""}`}
            aria-label={open ? `Collapse ${row.name}` : `Expand ${row.name}`}
            onClick={(e) => {
              e.stopPropagation();
              toggle(row.path);
            }}
          />
          <span className="tree-ico folder">
            <FolderIco />
          </span>
          <span className="tree-name">{row.name}</span>
          <span className="tree-count">{row.count}</span>
        </div>
      );
    }
    if (row.type === "tools") {
      return (
        <div
          key={row.key}
          className={`tree-row folder ${!selected && activeFolder === "__tools__" ? "active" : ""}`}
          style={pad}
          role="treeitem"
          tabIndex={-1}
          aria-expanded={toolsOpen}
          onClick={(e) => {
            e.currentTarget.focus();
            setActiveFolder("__tools__");
            setState({ selectedDeviceId: null });
          }}
          onDoubleClick={() => toggle(TOOLS_COLLAPSE)}
        >
          <button
            type="button"
            className={`chev ${toolsOpen ? "open" : ""}`}
            aria-label={toolsOpen ? "Collapse Tools" : "Expand Tools"}
            onClick={(e) => {
              e.stopPropagation();
              toggle(TOOLS_COLLAPSE);
            }}
          />
          <span className="tree-ico folder">
            <FolderIco />
          </span>
          <span className="tree-name">Tools</span>
        </div>
      );
    }
    if (row.type === "tool") {
      const label =
        row.id === "pty" ? "Local PTY" : row.id === "pcap" ? "Packet capture" : "Edgeshark";
      const meta =
        row.id === "pty" ? "host shell" : row.id === "pcap" ? "pcap / tcpdump" : "127.0.0.1:5001";
      return (
        <button
          key={row.key}
          type="button"
          className="tree-row device"
          style={pad}
          onClick={() => {
            if (row.id === "pty") void openLocal();
            else openPcapPane();
          }}
        >
          <span className="chev spacer" />
          <span className="tree-ico host">
            <HostIco />
          </span>
          <span className="tree-name">{label}</span>
          <span className="tree-host">{meta}</span>
        </button>
      );
    }
    const d = row.device;
    const host = deviceHost(d);
    return (
      <div
        key={row.key}
        className={`tree-row device ${selected === d.id ? "selected" : ""}`}
        style={pad}
        role="treeitem"
        tabIndex={-1}
        aria-selected={selected === d.id}
        draggable
        title={host || d.name}
        onDragStart={(e) => {
          e.dataTransfer.setData("text/late-device", d.id);
          e.dataTransfer.effectAllowed = "move";
        }}
        onClick={(e) => {
          e.currentTarget.focus();
          selectDevice(d);
        }}
        onDoubleClick={() => connect(d)}
        onContextMenu={(e) => openMenu({ kind: "device", id: d.id, x: 0, y: 0 }, e)}
      >
        <span className="chev spacer" />
        <span className={`tree-ico host ${d.kind}`}>
          <DeviceIco kind={d.kind} />
        </span>
        <span className="tree-name">{d.name}</span>
        {host ? <span className="tree-host">{host}</span> : null}
        <span className="tree-kind">{kindLabel(d.kind)}</span>
      </div>
    );
  }

  return (
    <aside
      className="sidebar"
      onKeyDown={(e) => {
        const t = e.target as HTMLElement;
        if (t.closest("input, textarea, select, .folder-dlg, .new-pop, .folder-menu")) return;
        onTreeKey(e);
      }}
      onClick={(e) => {
        if ((e.target as HTMLElement).closest(".folder-dlg, .folder-menu, .new-pop")) return;
        setMenu(null);
      }}
    >
      <div className="side-head">Sessions</div>
      <div className="sess-toolbar">
        <button
          type="button"
          className="sess-tb"
          title="Connect"
          disabled={!selectedDevice}
          onClick={() => selectedDevice && connect(selectedDevice)}
        >
          <svg viewBox="0 0 16 16" aria-hidden>
            <path
              d="M4 8h5M8 5l3 3-3 3M3 4v8"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.4"
            />
          </svg>
        </button>
        <button
          type="button"
          className="sess-tb"
          title="New folder"
          onClick={(e) => {
            e.stopPropagation();
            askNewFolder(activeFolder === "__tools__" ? "" : activeFolder);
          }}
        >
          <svg viewBox="0 0 16 16" aria-hidden>
            <path fill="currentColor" d="M1.5 3.5h4.2l.8 1.5H12.5v2h-1V6H2.5v7h8v1h-9z" />
            <path d="M12 9v6M9 12h6" stroke="currentColor" strokeWidth="1.4" />
          </svg>
        </button>
        <span className="new-menu">
          <button
            type="button"
            className="sess-tb"
            title="New session"
            onClick={(e) => {
              e.stopPropagation();
              setNewOpen((v) => !v);
            }}
          >
            <svg viewBox="0 0 16 16" aria-hidden>
              <path d="M8 3v10M3 8h10" stroke="currentColor" strokeWidth="1.5" />
            </svg>
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
                  askNewFolder(activeFolder === "__tools__" ? "" : activeFolder);
                }}
              >
                New folder
              </button>
              <button
                type="button"
                onClick={() => {
                  setNewOpen(false);
                  startDeviceEditor(undefined, "ssh", activeFolder === "__tools__" ? "" : activeFolder);
                }}
              >
                SSH session
              </button>
              <button
                type="button"
                onClick={() => {
                  setNewOpen(false);
                  startDeviceEditor(undefined, "serial", activeFolder === "__tools__" ? "" : activeFolder);
                }}
              >
                Serial console
              </button>
              <button
                type="button"
                onClick={() => {
                  setNewOpen(false);
                  startDeviceEditor(undefined, "api", activeFolder === "__tools__" ? "" : activeFolder);
                }}
              >
                API controller
              </button>
              <button
                type="button"
                onClick={() => {
                  setNewOpen(false);
                  void openLocal();
                }}
              >
                Local shell
              </button>
              <button
                type="button"
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
        <button
          type="button"
          className="sess-tb"
          title="Session properties"
          disabled={!selectedDevice}
          onClick={() => selectedDevice && startDeviceEditor(selectedDevice)}
        >
          <svg viewBox="0 0 16 16" aria-hidden>
            <rect x="3" y="3" width="10" height="10" rx="1.2" fill="none" stroke="currentColor" strokeWidth="1.3" />
            <path d="M6 7h4M6 9.5h3" stroke="currentColor" strokeWidth="1.3" />
          </svg>
        </button>
        <button
          type="button"
          className="sess-tb"
          title="Delete"
          disabled={!selectedDevice && !activeFolder}
          onClick={() => {
            if (selectedDevice) void deleteDevice(selectedDevice.id);
            else if (activeFolder && activeFolder !== "__tools__") confirmDelete(activeFolder);
          }}
        >
          <svg viewBox="0 0 16 16" aria-hidden>
            <path d="M4 5h8M6 5V3.5h4V5M5.5 5l.5 8h4l.5-8" fill="none" stroke="currentColor" strokeWidth="1.3" />
          </svg>
        </button>
        <span className="sess-tb-gap" />
        <button type="button" className="sess-tb" title="Expand all folders" onClick={expandAll}>
          <svg viewBox="0 0 16 16" aria-hidden>
            <path d="M3 5h10M3 8h10M3 11h10" stroke="currentColor" strokeWidth="1.3" />
          </svg>
        </button>
        <button type="button" className="sess-tb" title="Collapse all folders" onClick={collapseAll}>
          <svg viewBox="0 0 16 16" aria-hidden>
            <path d="M3 8h10" stroke="currentColor" strokeWidth="1.3" />
          </svg>
        </button>
      </div>
      <input
        ref={filterRef}
        className="search"
        placeholder="Filter by session name (Alt+I)"
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
        role="tree"
        onDragOver={(e) => {
          e.preventDefault();
          e.dataTransfer.dropEffect = "move";
        }}
        onDrop={(e) => {
          if ((e.target as HTMLElement).closest(".tree-row")) return;
          onDropFolder(null, e);
        }}
      >
        {rows.map(renderRow)}
      </div>
      {selectedDevice && (
        <div className="sess-connect">
          <button className="primary" type="button" onClick={() => connect(selectedDevice)}>
            Connect
          </button>
          {selectedDevice.kind === "ssh" && (
            <button className="ghost" type="button" onClick={() => connect(selectedDevice, "sftp")}>
              SFTP
            </button>
          )}
        </div>
      )}
      <div className="side-actions">
        <button className="ghost" type="button" onClick={() => setState({ keysOpen: true })}>
          Keys
        </button>
        <button className="ghost" type="button" onClick={() => setState({ authOpen: true })}>
          Auth
        </button>
        <button className="ghost" type="button" onClick={() => setState({ importOpen: true })}>
          Import
        </button>
        <button className="ghost" type="button" onClick={() => setState({ collectionsOpen: true })}>
          Collections
        </button>
        <button className="ghost" type="button" onClick={() => setState({ settingsOpen: true })}>
          Settings
        </button>
      </div>
      {menu?.kind === "folder" && (
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
          <button type="button" onClick={() => askRename(menu.path)}>
            Rename
          </button>
          <button type="button" onClick={() => confirmDelete(menu.path)}>
            Delete folder
          </button>
        </div>
      )}
      {menu?.kind === "device" && (
        <div
          className="folder-menu"
          style={{ left: menu.x, top: menu.y }}
          onClick={(e) => e.stopPropagation()}
          onMouseDown={(e) => e.stopPropagation()}
        >
          <button
            type="button"
            onClick={() => {
              const d = inventory.devices.find((x) => x.id === menu.id);
              setMenu(null);
              if (d) connect(d);
            }}
          >
            Connect
          </button>
          {inventory.devices.find((x) => x.id === menu.id)?.kind === "ssh" && (
            <button
              type="button"
              onClick={() => {
                const d = inventory.devices.find((x) => x.id === menu.id);
                setMenu(null);
                if (d) connect(d, "sftp");
              }}
            >
              SFTP
            </button>
          )}
          <button
            type="button"
            onClick={() => {
              const d = inventory.devices.find((x) => x.id === menu.id);
              setMenu(null);
              if (d) startDeviceEditor(d);
            }}
          >
            Properties
          </button>
          <button
            type="button"
            onClick={() => {
              setMenu(null);
              void deleteDevice(menu.id);
            }}
          >
            Delete session
          </button>
        </div>
      )}
    </aside>
  );
}
