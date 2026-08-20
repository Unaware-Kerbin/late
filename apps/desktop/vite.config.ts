import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";

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
  plugins: [react(), lateTokenPlugin()],
  base: "./",
  server: {
    host: "127.0.0.1",
    port: 5173,
    strictPort: true,
  },
  clearScreen: false,
  envPrefix: ["VITE_", "TAURI_"],
});
