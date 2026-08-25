import { spawn, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";

const desktopDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(desktopDir, "../..");

function lateConfigDir(): string {
  const home = os.homedir();
  if (process.platform === "win32") {
    return path.join(process.env.APPDATA || path.join(home, "AppData", "Roaming"), "late");
  }
  if (process.platform === "darwin") {
    return path.join(home, "Library", "Application Support", "late");
  }
  return path.join(process.env.XDG_CONFIG_HOME || path.join(home, ".config"), "late");
}

function lateDevOrigin(origin: string | undefined): boolean {
  if (!origin) return false;
  try {
    const u = new URL(origin);
    const host = u.hostname.replace(/^\[|\]$/g, "");
    const loopback = host === "127.0.0.1" || host === "localhost" || host === "::1";
    return loopback && (u.port === "5173" || u.port === "4173" || u.port === "1420");
  } catch {
    return false;
  }
}

function sameOriginFetch(req: { headers: Record<string, string | string[] | undefined> }): boolean {
  const site = req.headers["sec-fetch-site"];
  return site === "same-origin";
}

function httpOk(url: string, timeoutMs = 800): Promise<boolean> {
  return new Promise((resolve) => {
    const req = http.get(url, (res) => {
      res.resume();
      resolve(Boolean(res.statusCode && res.statusCode < 500));
    });
    req.on("error", () => resolve(false));
    req.setTimeout(timeoutMs, () => {
      req.destroy();
      resolve(false);
    });
  });
}

function findDevDaemonBin(): string | null {
  const name = process.platform === "win32" ? "late-daemon.exe" : "late-daemon";
  const built = [
    path.join(repoRoot, "target", "debug", name),
    path.join(repoRoot, "target", "release", name),
  ].filter((p) => fs.existsSync(p));
  if (built.length) {
    built.sort((a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs);
    return built[0];
  }
  return null;
}

function lateDaemonPlugin(): Plugin {
  let child: ChildProcess | undefined;
  return {
    name: "late-daemon-boot",
    async configureServer() {
      if (await httpOk("http://127.0.0.1:7420/health")) return;
      const bin = findDevDaemonBin();
      if (!bin) {
        console.warn("late: daemon binary not found; UI will not connect until cargo build -p late-daemon");
        return;
      }
      let log: number | "ignore" = "ignore";
      try {
        log = fs.openSync(path.join(os.tmpdir(), "late-daemon.log"), "a");
      } catch {
        log = "ignore";
      }
      console.log("late: starting daemon", bin);
      const policies = path.join(desktopDir, "resources", "policies");
      const env = { ...process.env };
      if (fs.existsSync(policies)) env.LATE_POLICIES_DIR = policies;
      child = spawn(bin, ["--bind", "127.0.0.1:7420"], {
        stdio: log === "ignore" ? "ignore" : ["ignore", log, log],
        windowsHide: true,
        env,
      });
      child.on("error", (err) => console.error("late: daemon spawn failed", err));
    },
    closeBundle() {
      child?.kill();
    },
  };
}

function lateTokenPlugin(): Plugin {
  return {
    name: "late-token",
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        if (req.url?.split("?")[0] !== "/__late_token") {
          next();
          return;
        }
        const origin = typeof req.headers.origin === "string" ? req.headers.origin : undefined;
        const referer = typeof req.headers.referer === "string" ? req.headers.referer : undefined;
        let refererOrigin: string | undefined;
        try {
          refererOrigin = referer ? new URL(referer).origin : undefined;
        } catch {
          refererOrigin = undefined;
        }
        if ((!lateDevOrigin(origin) && !lateDevOrigin(refererOrigin)) || !sameOriginFetch(req)) {
          res.statusCode = 403;
          res.end("");
          return;
        }
        res.setHeader("Cache-Control", "no-store");
        res.setHeader("Content-Type", "text/plain; charset=utf-8");
        try {
          res.end(fs.readFileSync(path.join(lateConfigDir(), "sidecar.token"), "utf8").trim());
        } catch {
          res.statusCode = 404;
          res.end("");
        }
      });
    },
  };
}

export default defineConfig({
  plugins: [react(), lateTokenPlugin(), lateDaemonPlugin()],
  base: "./",
  server: {
    host: "127.0.0.1",
    port: 5173,
    strictPort: true,
  },
  clearScreen: false,
  envPrefix: ["VITE_", "TAURI_"],
});
