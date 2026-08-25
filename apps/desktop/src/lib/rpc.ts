import { lateLocalToken } from "./localToken";

export const DAEMON_HTTP = "http://127.0.0.1:7420";
export const DAEMON_WS = "ws://127.0.0.1:7420/ws";

type Pending = {
  resolve: (v: unknown) => void;
  reject: (e: Error) => void;
};

export type SessionBytesHandler = (sessionId: string, bytes: Uint8Array) => void;

function b64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

export function bytesToB64(bytes: Uint8Array): string {
  let s = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    s += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(s);
}

export function textToB64(text: string): string {
  return bytesToB64(new TextEncoder().encode(text));
}

export class RpcError extends Error {
  code?: number;
  data?: unknown;
  constructor(message: string, code?: number, data?: unknown) {
    super(message);
    this.code = code;
    this.data = data;
  }
}

export function isHostKeyError(err: unknown): { mismatch: boolean; host: string; presented?: string; pinned?: string } | null {
  const msg = err instanceof Error ? err.message : String(err);
  const data = err instanceof RpcError ? (err.data as Record<string, unknown> | undefined) : undefined;
  const lower = msg.toLowerCase();
  const mismatch =
    /mismatch|changed/.test(lower) || data?.kind === "host_key_mismatch" || data?.code === "host_key_mismatch";
  const untrusted =
    /untrusted|unknown|tofu|host key/.test(lower) ||
    data?.kind === "host_key_untrusted" ||
    data?.code === "host_key_untrusted";
  if (!mismatch && !untrusted) return null;
  const host = String(data?.host ?? /for ([^\s:]+)/.exec(msg)?.[1] ?? "");
  return {
    mismatch,
    host,
    presented: data?.presented ? String(data.presented) : /presented[:\s]+(\S+)/i.exec(msg)?.[1],
    pinned: data?.pinned ? String(data.pinned) : /pinned[:\s]+(\S+)/i.exec(msg)?.[1],
  };
}

class DaemonRpc {
  private ws: WebSocket | null = null;
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private sessionHandlers = new Set<SessionBytesHandler>();
  private statusHandlers = new Set<(ok: boolean, err?: string) => void>();
  private closedHandlers = new Set<(sessionId: string, reason?: string) => void>();
  private opening: Promise<void> | null = null;
  connected = false;
  lastError: string | null = null;

  onBytes(h: SessionBytesHandler): () => void {
    this.sessionHandlers.add(h);
    return () => this.sessionHandlers.delete(h);
  }

  onStatus(h: (ok: boolean, err?: string) => void): () => void {
    this.statusHandlers.add(h);
    return () => this.statusHandlers.delete(h);
  }

  onClosed(h: (sessionId: string, reason?: string) => void): () => void {
    this.closedHandlers.add(h);
    return () => this.closedHandlers.delete(h);
  }

  async health(): Promise<boolean> {
    try {
      const r = await fetch(`${DAEMON_HTTP}/health`, { signal: AbortSignal.timeout(1500) });
      return r.ok;
    } catch {
      return false;
    }
  }

  fail(msg: string, reject?: (e: Error) => void) {
    this.opening = null;
    this.lastError = msg;
    this.connected = false;
    this.statusHandlers.forEach((h) => h(false, msg));
    reject?.(new Error(msg));
  }

  connect(): Promise<void> {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) return Promise.resolve();
    if (this.opening) return this.opening;
    this.opening = new Promise((resolve, reject) => {
      void (async () => {
        const token = await lateLocalToken();
        const ws = token
          ? new WebSocket(DAEMON_WS, [`late.${token}`])
          : new WebSocket(DAEMON_WS);
        const timer = window.setTimeout(() => {
          ws.close();
          reject(new Error("daemon websocket timeout"));
        }, 4000);
        ws.onopen = () => {
          window.clearTimeout(timer);
          this.ws = ws;
          this.connected = true;
          this.lastError = null;
          this.statusHandlers.forEach((h) => h(true));
          resolve();
        };
        ws.onmessage = (ev) => this.onMessage(String(ev.data));
        ws.onclose = () => {
          this.ws = null;
          this.opening = null;
          this.connected = false;
          this.lastError = "daemon disconnected";
          this.statusHandlers.forEach((h) => h(false, this.lastError ?? undefined));
          for (const [, p] of this.pending) p.reject(new Error("daemon websocket closed"));
          this.pending.clear();
        };
        ws.onerror = () => {
          window.clearTimeout(timer);
          const msg = token
            ? "Could not open the daemon websocket. If the daemon is running, restart Late so it can pick up the local token. From this repo use npm start."
            : "Could not open the daemon websocket. From this repo run npm start (rebuilds and starts late-daemon).";
          this.fail(msg, reject);
        };
      })().catch((err) => {
        this.opening = null;
        reject(err instanceof Error ? err : new Error(String(err)));
      });
    });
    return this.opening;
  }

  private onMessage(raw: string) {
    let msg: Record<string, unknown>;
    try {
      msg = JSON.parse(raw) as Record<string, unknown>;
    } catch {
      return;
    }
    const event = (msg.event ?? msg.method) as string | undefined;
    if (event === "session.data" || event === "sessionData" || event === "session.output") {
      const sessionId = String(msg.sessionId ?? msg.session_id ?? "");
      const data = String(msg.data ?? (msg as { params?: { data?: string } }).params?.data ?? "");
      if (sessionId && data) {
        const bytes = b64ToBytes(data);
        this.sessionHandlers.forEach((h) => h(sessionId, bytes));
      }
      return;
    }
    if (event === "session.closed" || event === "sessionClosed") {
      const sessionId = String(msg.sessionId ?? msg.session_id ?? "");
      const reason = msg.reason ? String(msg.reason) : undefined;
      if (sessionId) this.closedHandlers.forEach((h) => h(sessionId, reason));
      return;
    }
    if (typeof msg.id === "number" && this.pending.has(msg.id)) {
      const p = this.pending.get(msg.id)!;
      this.pending.delete(msg.id);
      if (msg.error) {
        const err = msg.error as { message?: string; code?: number; data?: unknown };
        p.reject(new RpcError(err.message ?? JSON.stringify(msg.error), err.code, err.data));
      } else {
        p.resolve(msg.result);
      }
    }
  }

  async call<T = unknown>(method: string, params?: unknown, timeoutMs = 30_000): Promise<T> {
    await this.connect();
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve: (v) => resolve(v as T), reject });
      this.ws!.send(JSON.stringify({ id, method, params: params ?? {} }));
      window.setTimeout(() => {
        if (this.pending.has(id)) {
          this.pending.delete(id);
          reject(new RpcError(`RPC timeout: ${method}`));
        }
      }, timeoutMs);
    });
  }
}

export const rpc = new DaemonRpc();
