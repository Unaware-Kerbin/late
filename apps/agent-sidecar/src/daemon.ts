import WebSocket from "ws";
import { DAEMON_HTTP, DAEMON_WS } from "./types.js";

type Pending = {
  resolve: (v: unknown) => void;
  reject: (e: Error) => void;
};

export class DaemonClient {
  private ws: WebSocket | null = null;
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private opening: Promise<void> | null = null;

  async health(): Promise<boolean> {
    try {
      const r = await fetch(`${DAEMON_HTTP}/health`, { signal: AbortSignal.timeout(1500) });
      return r.ok;
    } catch {
      return false;
    }
  }

  private ensure(): Promise<void> {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) return Promise.resolve();
    if (this.opening) return this.opening;
    this.opening = new Promise((resolve, reject) => {
      const ws = new WebSocket(DAEMON_WS);
      const timer = setTimeout(() => {
        ws.close();
        reject(new Error(`daemon WS timeout (${DAEMON_WS})`));
      }, 4000);
      ws.on("open", () => {
        clearTimeout(timer);
        this.ws = ws;
        resolve();
      });
      ws.on("message", (raw) => this.onMessage(String(raw)));
      ws.on("close", () => {
        this.ws = null;
        this.opening = null;
        for (const [, p] of this.pending) p.reject(new Error("daemon WS closed"));
        this.pending.clear();
      });
      ws.on("error", (err) => {
        clearTimeout(timer);
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
    if (typeof msg.id === "number" && this.pending.has(msg.id)) {
      const p = this.pending.get(msg.id)!;
      this.pending.delete(msg.id);
      if (msg.error) {
        const err = msg.error as { message?: string };
        p.reject(new Error(err.message ?? JSON.stringify(msg.error)));
      } else {
        p.resolve(msg.result);
      }
    }
  }

  async call<T = unknown>(method: string, params?: unknown): Promise<T> {
    await this.ensure();
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve: (v) => resolve(v as T), reject });
      this.ws!.send(JSON.stringify({ id, method, params: params ?? {} }), (err) => {
        if (err) {
          this.pending.delete(id);
          reject(err);
        }
      });
      setTimeout(() => {
        if (this.pending.has(id)) {
          this.pending.delete(id);
          reject(new Error(`RPC timeout: ${method}`));
        }
      }, 30_000);
    });
  }
}

export const daemon = new DaemonClient();
