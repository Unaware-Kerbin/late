import { useCallback, useEffect, useState } from "react";
import { rpc } from "../lib/rpc";
import { toast } from "../store";
import type { PaneState, SftpEntry } from "../types";

type FileRow = SftpEntry & { isDir: boolean };

function asRecord(v: unknown): Record<string, unknown> | null {
  return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
}

function coerceEntry(raw: unknown): FileRow | null {
  const o = asRecord(raw);
  if (!o) return null;
  const name = String(o.name ?? "").trim();
  if (!name) return null;
  const path = String(o.path ?? o.fullPath ?? name);
  const isDir = Boolean(o.is_dir ?? o.isDir ?? o.dir);
  const size = Number(o.size ?? 0) || 0;
  return { name, path, is_dir: isDir, isDir, size };
}

function coerceListing(raw: unknown, fallbackPath: string): { path: string; entries: FileRow[] } {
  const arr = Array.isArray(raw)
    ? raw
    : asRecord(raw)
      ? ((asRecord(raw)!.entries ?? asRecord(raw)!.files ?? []) as unknown[])
      : [];
  const entries = (Array.isArray(arr) ? arr : []).map(coerceEntry).filter((e): e is FileRow => !!e);
  const rec = asRecord(raw);
  const path = rec && typeof rec.path === "string" && rec.path ? rec.path : fallbackPath;
  return { path, entries };
}

function parentOf(path: string): string {
  const trimmed = path.replace(/[/\\]+$/, "");
  if (!trimmed || trimmed === "/" || /^[A-Za-z]:\\?$/.test(trimmed)) return path.startsWith("/") ? "/" : trimmed || "/";
  const idx = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  if (idx <= 0) return "/";
  if (idx === 2 && trimmed[1] === ":") return trimmed.slice(0, 3);
  return trimmed.slice(0, idx) || "/";
}

function joinPath(dir: string, name: string): string {
  if (!name || name === "." || name === ".." || /[/\\]/.test(name)) {
    throw new Error("invalid file name");
  }
  if (!dir || dir === "/") return `/${name}`.replace(/\/+/g, "/");
  if (dir.endsWith("/") || dir.endsWith("\\")) return `${dir}${name}`;
  const sep = dir.includes("\\") && !dir.includes("/") ? "\\" : "/";
  return `${dir}${sep}${name}`;
}

function fmtSize(n: number, dir: boolean): string {
  if (dir) return "";
  if (!Number.isFinite(n) || n < 0) return "";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function errText(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export function SftpPane({ pane }: { pane: PaneState }) {
  const sessionId = pane.session?.id;
  const [localPath, setLocalPath] = useState("");
  const [remotePath, setRemotePath] = useState("/");
  const [localDraft, setLocalDraft] = useState("");
  const [remoteDraft, setRemoteDraft] = useState("/");
  const [local, setLocal] = useState<FileRow[]>([]);
  const [remote, setRemote] = useState<FileRow[]>([]);
  const [localSel, setLocalSel] = useState<FileRow | null>(null);
  const [remoteSel, setRemoteSel] = useState<FileRow | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [localErr, setLocalErr] = useState<string | null>(null);
  const [remoteErr, setRemoteErr] = useState<string | null>(null);

  const loadLocal = useCallback(async (path: string) => {
    try {
      const raw = await rpc.call<unknown>("sftp.local", { path }, 20_000);
      const listing = coerceListing(raw, path);
      setLocal(listing.entries);
      setLocalPath(listing.path);
      setLocalDraft(listing.path);
      setLocalSel(null);
      setLocalErr(null);
    } catch (err) {
      setLocal([]);
      setLocalErr(errText(err));
    }
  }, []);

  const loadRemote = useCallback(
    async (path: string) => {
      if (!sessionId) {
        setRemote([]);
        setRemoteErr(null);
        return;
      }
      try {
        const raw = await rpc.call<unknown>(
          "sftp.list",
          { sessionId, session_id: sessionId, path },
          45_000,
        );
        const listing = coerceListing(raw, path);
        setRemote(listing.entries);
        setRemotePath(listing.path || path);
        setRemoteDraft(listing.path || path);
        setRemoteSel(null);
        setRemoteErr(null);
      } catch (err) {
        setRemote([]);
        setRemoteErr(errText(err));
        toast("error", errText(err));
      }
    },
    [sessionId],
  );

  useEffect(() => {
    void loadLocal("");
  }, [loadLocal]);

  useEffect(() => {
    if (sessionId) void loadRemote("/");
    else {
      setRemote([]);
      setRemoteErr(null);
    }
  }, [sessionId, loadRemote]);

  async function transfer(kind: "upload" | "download") {
    if (!sessionId) {
      toast("error", "Connect an SFTP session from the sidebar first.");
      return;
    }
    const src = kind === "upload" ? localSel : remoteSel;
    if (!src || src.isDir) {
      toast("error", kind === "upload" ? "Select a local file to upload." : "Select a remote file to download.");
      return;
    }
    const destDir = kind === "upload" ? remotePath : localPath;
    if (!destDir) {
      toast("error", "Open a destination folder first.");
      return;
    }
    const dest = joinPath(destDir, src.name);
    setBusy(kind);
    try {
      if (kind === "upload") {
        await rpc.call("sftp.upload", { sessionId, session_id: sessionId, local: src.path, remote: dest }, 120_000);
        toast("ok", `uploaded ${src.name}`);
        await loadRemote(remotePath);
      } else {
        await rpc.call("sftp.download", { sessionId, session_id: sessionId, remote: src.path, local: dest }, 120_000);
        toast("ok", `downloaded ${src.name}`);
        await loadLocal(localPath);
      }
    } catch (err) {
      toast("error", errText(err));
    } finally {
      setBusy(null);
    }
  }

  function openLocal(e: FileRow) {
    if (e.isDir) void loadLocal(e.path);
    else setLocalSel(e);
  }

  function openRemote(e: FileRow) {
    if (!sessionId) return;
    if (e.isDir) void loadRemote(e.path);
    else setRemoteSel(e);
  }

  return (
    <div className="sftp-pane">
      {!sessionId && (
        <div className="sftp-banner">SFTP session not connected — local files still work. Open SFTP from the sidebar to browse the remote host.</div>
      )}
      <div className="dual sftp-dual">
        <Column
          title="Local"
          path={localDraft}
          onPath={setLocalDraft}
          onGo={() => void loadLocal(localDraft.trim() || localPath)}
          onUp={() => void loadLocal(parentOf(localPath || "/"))}
          entries={local}
          selected={localSel}
          onSelect={setLocalSel}
          onOpen={openLocal}
          error={localErr}
          disabled={!!busy}
        />
        <Column
          title="Remote"
          path={remoteDraft}
          onPath={setRemoteDraft}
          onGo={() => sessionId && void loadRemote(remoteDraft.trim() || remotePath)}
          onUp={() => sessionId && void loadRemote(parentOf(remotePath || "/"))}
          entries={remote}
          selected={remoteSel}
          onSelect={setRemoteSel}
          onOpen={openRemote}
          error={remoteErr}
          disabled={!sessionId || !!busy}
          empty={!sessionId ? "No remote session" : undefined}
        />
      </div>
      <div className="sftp-actions">
        <button className="primary" disabled={!sessionId || !localSel || localSel.isDir || !!busy} onClick={() => void transfer("upload")}>
          {busy === "upload" ? "Uploading…" : "Upload →"}
        </button>
        <button className="ghost" disabled={!sessionId || !remoteSel || remoteSel.isDir || !!busy} onClick={() => void transfer("download")}>
          {busy === "download" ? "Downloading…" : "← Download"}
        </button>
        <button className="ghost" disabled={!!busy} onClick={() => { void loadLocal(localPath); if (sessionId) void loadRemote(remotePath); }}>
          Refresh
        </button>
        <span className="sftp-hint">Double-click a folder to open it. Agent tools cannot use SFTP.</span>
      </div>
    </div>
  );
}

function Column({
  title,
  path,
  onPath,
  onGo,
  onUp,
  entries,
  selected,
  onSelect,
  onOpen,
  error,
  disabled,
  empty,
}: {
  title: string;
  path: string;
  onPath: (v: string) => void;
  onGo: () => void;
  onUp: () => void;
  entries: FileRow[];
  selected: FileRow | null;
  onSelect: (e: FileRow) => void;
  onOpen: (e: FileRow) => void;
  error: string | null;
  disabled?: boolean;
  empty?: string;
}) {
  return (
    <div className="sftp-col">
      <div className="sftp-path">
        <span className="sftp-side">{title}</span>
        <button className="ghost" type="button" disabled={disabled} onClick={onUp} title="Parent folder">
          ..
        </button>
        <input
          value={path}
          disabled={disabled}
          onChange={(e) => onPath(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") onGo();
          }}
        />
        <button className="ghost" type="button" disabled={disabled} onClick={onGo}>
          Go
        </button>
      </div>
      <div className="list sftp-files">
        {error && <div className="sftp-err">{error}</div>}
        {empty && !error && <div className="sftp-empty">{empty}</div>}
        {!empty &&
          !error &&
          entries.map((e) => (
            <div
              key={e.path || e.name}
              className={`file${selected?.path === e.path ? " sel" : ""}`}
              title={e.path}
              onClick={() => onSelect(e)}
              onDoubleClick={() => onOpen(e)}
            >
              <span className="file-name">
                {e.isDir ? "▸" : "·"} {e.name}
              </span>
              <span className="file-size">{fmtSize(e.size, e.isDir)}</span>
            </div>
          ))}
        {!empty && !error && entries.length === 0 && <div className="sftp-empty">Empty folder</div>}
      </div>
    </div>
  );
}
