import { readLocalToken } from "./local-auth.js";
import { DAEMON_HTTP } from "./types.js";

type JsonRpc = {
  id?: unknown;
  result?: unknown;
  error?: { message?: string; code?: number; data?: unknown };
};

async function rpcToken(): Promise<string> {
  for (let i = 0; i < 20; i++) {
    const t = readLocalToken();
    if (t) return t;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error("missing local RPC token");
}

export class DaemonClient {
  private nextId = 1;

  async health(): Promise<boolean> {
    try {
      const r = await fetch(`${DAEMON_HTTP}/health`, { signal: AbortSignal.timeout(1500) });
      return r.ok;
    } catch {
      return false;
    }
  }

  async call<T = unknown>(method: string, params?: unknown): Promise<T> {
    const token = await rpcToken();
    const id = this.nextId++;
    const r = await fetch(`${DAEMON_HTTP}/rpc`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-Late-Token": token,
      },
      body: JSON.stringify({ id, method, params: params ?? {} }),
      signal: AbortSignal.timeout(30_000),
    });
    const text = await r.text();
    if (r.status === 401 || r.status === 403) {
      throw new Error("daemon RPC unauthorized — restart the sidecar so it can present the local token");
    }
    if (!r.ok) {
      throw new Error(`daemon RPC ${r.status}: ${text.slice(0, 240)}`);
    }
    let msg: JsonRpc;
    try {
      msg = JSON.parse(text) as JsonRpc;
    } catch {
      throw new Error(`daemon RPC non-JSON: ${text.slice(0, 240)}`);
    }
    if (msg.error) {
      throw new Error(msg.error.message ?? JSON.stringify(msg.error));
    }
    return msg.result as T;
  }
}

export const daemon = new DaemonClient();
