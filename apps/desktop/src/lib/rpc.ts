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

/** Daemon WS stdin/stdout frame: `sessionId\\0` + raw bytes. */
export function encodeTermDataFrame(sessionId: string, payload: Uint8Array): Uint8Array {
  const sid = new TextEncoder().encode(sessionId);
  const buf = new Uint8Array(sid.length + 1 + payload.length);
  buf.set(sid, 0);
  buf[sid.length] = 0;
  buf.set(payload, sid.length + 1);
  return buf;
}

/** Daemon WS stdin frame: `sessionId\\0` + UTF-8 bytes. */
export function encodeTermInputFrame(sessionId: string, text: string): Uint8Array {
  return encodeTermDataFrame(sessionId, new TextEncoder().encode(text));
}

export function parseTermDataFrame(buf: Uint8Array): { sessionId: string; bytes: Uint8Array } | null {
  const z = buf.indexOf(0);
  if (z <= 0 || z > 80) return null;
  const sessionId = new TextDecoder().decode(buf.subarray(0, z));
  if (![...sessionId].every((c) => /[0-9a-fA-F-]/.test(c))) return null;
  return { sessionId, bytes: buf.subarray(z + 1) };
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

export function isCancelled(err: unknown): boolean {
  const msg = err instanceof Error ? err.message : String(err);
  return /^(cancelled|aborted)$/i.test(msg.trim()) || /rpc cancelled/i.test(msg);
}

export function isConnectError(err: unknown): { host: string; reason: string; cause: string } | null {
  if (isHostKeyError(err) || isCancelled(err)) return null;
  const msg = err instanceof Error ? err.message : String(err);
  const data = err instanceof RpcError ? (err.data as Record<string, unknown> | undefined) : undefined;
  const code = String(data?.code ?? data?.kind ?? "");
  const structured = code === "unable_to_connect" || data?.kind === "unable_to_connect";
  const rpcOpenTimeout = /rpc timeout:\s*session\.open/i.test(msg);
  const timeout = /timed out|connection timed out/i.test(msg) || rpcOpenTimeout;
  const refused = /connection refused/i.test(msg);
  const unreach = /no route to host|network is unreachable|host is unreachable/i.test(msg);
  const auth = /permission denied|authentication failed|too many authentication/i.test(msg);
  const banner = /no ssh banner|unable to connect/i.test(msg);
  if (!structured && !timeout && !refused && !unreach && !auth && !banner) return null;
  const cause = String(
    data?.cause ??
      (timeout ? "timeout" : refused ? "refused" : unreach ? "unreachable" : auth ? "auth" : "failed"),
  );
  const canned =
    cause === "timeout"
      ? "connection timed out"
      : cause === "refused"
        ? "connection refused"
        : cause === "unreachable"
          ? "no route to host"
          : cause === "auth"
            ? "authentication failed"
            : "unable to connect";
  const reason = String(data?.reason ?? canned);
  const host = String(data?.host ?? "");
  return { host, reason, cause };
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
        ws.binaryType = "arraybuffer";
        ws.onopen = () => {
          window.clearTimeout(timer);
          this.ws = ws;
          this.connected = true;
          this.lastError = null;
          this.statusHandlers.forEach((h) => h(true));
          resolve();
        };
        ws.onmessage = (ev) => {
          if (typeof ev.data !== "string") {
            this.onBinary(ev.data);
            return;
          }
          this.onMessage(ev.data);
        };
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

  private dispatchBytes(sessionId: string, bytes: Uint8Array) {
    if (!sessionId || !bytes.length) return;
    this.sessionHandlers.forEach((h) => h(sessionId, bytes));
  }

  private onBinary(data: unknown) {
    let buf: Uint8Array | null = null;
    if (data instanceof ArrayBuffer) buf = new Uint8Array(data);
    else if (data instanceof Uint8Array) buf = data;
    if (!buf) return;
    const parsed = parseTermDataFrame(buf);
    if (parsed) this.dispatchBytes(parsed.sessionId, parsed.bytes);
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
      if (sessionId && data) this.dispatchBytes(sessionId, b64ToBytes(data));
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

  async call<T = unknown>(
    method: string,
    params?: unknown,
    timeoutOrOpts: number | { timeoutMs?: number; signal?: AbortSignal } = 30_000,
  ): Promise<T> {
    const timeoutMs = typeof timeoutOrOpts === "number" ? timeoutOrOpts : (timeoutOrOpts.timeoutMs ?? 30_000);
    const signal = typeof timeoutOrOpts === "number" ? undefined : timeoutOrOpts.signal;
    await this.connect();
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
    let timer = 0;
    const finish = (fn: () => void) => {
      signal?.removeEventListener("abort", onAbort);
      window.clearTimeout(timer);
      fn();
    };
    const onAbort = () => {
      if (!this.pending.has(id)) return;
      this.pending.delete(id);
      finish(() => reject(new RpcError("cancelled")));
    };
    if (signal?.aborted) {
      reject(new RpcError("cancelled"));
      return;
    }
    this.pending.set(id, {
      resolve: (v) => finish(() => resolve(v as T)),
      reject: (e) => finish(() => reject(e)),
    });
    this.ws!.send(JSON.stringify({ id, method, params: params ?? {} }));
    signal?.addEventListener("abort", onAbort, { once: true });
    timer = window.setTimeout(() => {
      if (this.pending.has(id)) {
        this.pending.delete(id);
        finish(() => reject(new RpcError(`RPC timeout: ${method}`)));
      }
    }, timeoutMs);
    });
  }

  /** Keystroke path: binary WS frame, no JSON-RPC round-trip, no local echo. */
  async sendTermInput(sessionId: string, text: string): Promise<void> {
    await this.connect();
    const ws = this.ws;
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      throw new RpcError("daemon websocket closed");
    }
    ws.send(encodeTermInputFrame(sessionId, text));
  }
}

export const rpc = new DaemonRpc();
