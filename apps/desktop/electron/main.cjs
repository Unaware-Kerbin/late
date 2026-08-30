const { app, BrowserWindow, shell, ipcMain, Menu, clipboard, session } = require("electron");
const { spawn } = require("node:child_process");
const http = require("node:http");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { fileURLToPath } = require("node:url");

app.disableHardwareAcceleration();
app.commandLine.appendSwitch("disable-gpu");
app.commandLine.appendSwitch("disable-gpu-compositing");
app.commandLine.appendSwitch("disable-gpu-vsync");
app.commandLine.appendSwitch("log-level", "3");
if (process.platform === "linux") {
  app.commandLine.appendSwitch("ozone-platform", "x11");
}

const DEV_URL = process.env.LATE_DEV_URL || "http://127.0.0.1:5173";
const children = [];

function resource(...parts) {
  if (app.isPackaged) return path.join(process.resourcesPath, ...parts);
  return path.join(__dirname, "..", "resources", ...parts);
}

function repoRoot() {
  return path.join(__dirname, "..", "..", "..");
}

function httpOk(url, timeoutMs = 800) {
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

function openChildLog(name) {
  try {
    return fs.openSync(path.join(os.tmpdir(), name), "a");
  } catch {
    return null;
  }
}

function findDaemonBin() {
  const name = process.platform === "win32" ? "late-daemon.exe" : "late-daemon";
  if (app.isPackaged) {
    const bundled = resource("bin", name);
    return fs.existsSync(bundled) ? bundled : null;
  }
  const built = [
    path.join(repoRoot(), "target", "debug", name),
    path.join(repoRoot(), "target", "release", name),
  ].filter((p) => fs.existsSync(p));
  if (built.length) {
    built.sort((a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs);
    return built[0];
  }
  // Unpackaged: never the Aug 20 bundled binary (no stage.* RPCs). cargo run instead.
  return null;
}

function lateConfigDir() {
  const home = os.homedir();
  if (process.platform === "win32") {
    return path.join(process.env.APPDATA || path.join(home, "AppData", "Roaming"), "late");
  }
  if (process.platform === "darwin") {
    return path.join(home, "Library", "Application Support", "late");
  }
  return path.join(process.env.XDG_CONFIG_HOME || path.join(home, ".config"), "late");
}

function readLocalToken() {
  try {
    return fs.readFileSync(path.join(lateConfigDir(), "sidecar.token"), "utf8").trim();
  } catch {
    return "";
  }
}

function waitHttp(url, timeoutMs = 20000) {
  const start = Date.now();
  return new Promise((resolve, reject) => {
    const tick = () => {
      const req = http.get(url, (res) => {
        res.resume();
        if (res.statusCode && res.statusCode < 500) {
          resolve();
          return;
        }
        retry();
      });
      req.on("error", retry);
      req.setTimeout(800, () => {
        req.destroy();
        retry();
      });
    };
    const retry = () => {
      if (Date.now() - start > timeoutMs) {
        reject(new Error(`timeout waiting for ${url}`));
        return;
      }
      setTimeout(tick, 250);
    };
    tick();
  });
}

function startChild(bin, args, extraEnv, logName) {
  if (!fs.existsSync(bin)) {
    console.warn("late: missing", bin);
    return null;
  }
  const log = logName ? openChildLog(logName) : null;
  const child = spawn(bin, args, {
    env: { ...process.env, ...extraEnv },
    stdio: log == null ? "ignore" : ["ignore", log, log],
    windowsHide: true,
  });
  child.on("error", (err) => console.error("late child error", bin, err));
  child.on("exit", (code, signal) => {
    if (code) console.error("late child exit", bin, code, signal || "");
  });
  children.push(child);
  return child;
}

function startNpmSidecar(extraEnv) {
  const npm = process.platform === "win32" ? "npm.cmd" : "npm";
  const child = spawn(npm, ["run", "start", "-w", "late-agent-sidecar"], {
    cwd: repoRoot(),
    env: { ...process.env, ...extraEnv },
    stdio: "ignore",
    windowsHide: true,
    shell: process.platform === "win32",
  });
  child.on("error", (err) => console.error("late sidecar npm error", err));
  children.push(child);
  return child;
}

function startDaemonCargo(env) {
  const cargo = process.platform === "win32" ? "cargo.exe" : "cargo";
  const log = openChildLog("late-daemon.log");
  const child = spawn(cargo, ["run", "-q", "-p", "late-daemon", "--", "--bind", "127.0.0.1:7420"], {
    cwd: repoRoot(),
    env: { ...process.env, ...env },
    stdio: log == null ? "ignore" : ["ignore", log, log],
    windowsHide: true,
    shell: process.platform === "win32",
  });
  child.on("error", (err) => console.error("late cargo daemon error", err));
  children.push(child);
  return child;
}

async function startBackend() {
  if (!(await httpOk("http://127.0.0.1:7420/health"))) {
    const daemonBin = findDaemonBin();
    const policies = resource("policies");
    const env = {};
    if (fs.existsSync(policies)) env.LATE_POLICIES_DIR = policies;
    if (daemonBin) {
      console.log("late: starting daemon", daemonBin);
      startChild(daemonBin, ["--bind", "127.0.0.1:7420"], env, "late-daemon.log");
    } else if (!app.isPackaged) {
      console.warn("late: no late-daemon binary; cargo run -p late-daemon");
      startDaemonCargo(env);
    } else {
      console.error("late: late-daemon binary not found. Build with: cargo build -p late-daemon");
    }
    await waitHttp("http://127.0.0.1:7420/health", app.isPackaged ? 20000 : 90000).catch((err) => {
      console.error("late: daemon did not come up", err);
    });
  }
  if (await httpOk("http://127.0.0.1:7430/health")) return;
  const extra = {
    LATE_SIDECAR_PORT: "7430",
    LATE_DAEMON_HTTP: "http://127.0.0.1:7420",
    LATE_DAEMON_WS: "ws://127.0.0.1:7420/ws",
    LATE_RPC_TOKEN: readLocalToken(),
  };
  const sidecarJs = resource("sidecar.cjs");
  if (app.isPackaged && fs.existsSync(sidecarJs)) {
    startChild(process.execPath, [sidecarJs], { ELECTRON_RUN_AS_NODE: "1", ...extra }, "late-sidecar.log");
  } else {
    startNpmSidecar(extra);
  }
  await waitHttp("http://127.0.0.1:7430/health").catch((err) => {
    console.error("late: sidecar did not come up", err);
  });
}

function stopBackend() {
  for (const child of children) {
    try {
      child.kill();
    } catch {
      /* ignore */
    }
  }
}

function packagedIndexPath() {
  return path.resolve(__dirname, "../dist/index.html");
}

function isAllowedRendererUrl(url) {
  try {
    const u = new URL(url);
    if (!app.isPackaged) {
      return u.origin === new URL(DEV_URL).origin;
    }
    if (u.protocol !== "file:") return false;
    const got = path.resolve(fileURLToPath(u));
    const want = packagedIndexPath();
    if (process.platform === "win32") {
      return got.toLowerCase() === want.toLowerCase();
    }
    return got === want;
  } catch {
    return false;
  }
}

function isLoopbackHttpHost(hostname) {
  const host = hostname.replace(/^\[|\]$/g, "").toLowerCase();
  return host === "127.0.0.1" || host === "localhost" || host === "::1";
}

/** https anywhere, or http/https on loopback. */
function isAllowedExternal(url) {
  try {
    const u = new URL(url);
    if (u.protocol === "https:") return true;
    if (u.protocol === "http:") return isLoopbackHttpHost(u.hostname);
    return false;
  } catch {
    return false;
  }
}

function ipcFrameUrl(event) {
  try {
    const frame = event.senderFrame;
    if (frame && typeof frame.url === "string" && frame.url) return frame.url;
  } catch {
    /* senderFrame throws if the frame is already gone */
  }
  try {
    const url = event.sender && typeof event.sender.getURL === "function" ? event.sender.getURL() : "";
    if (typeof url === "string" && url) return url;
  } catch {
    /* webContents may already be destroyed */
  }
  return "";
}

/** Chromium 152 asks for this before renderer WebSocket/fetch to the loopback daemon. */
function isAllowedWebPermission(permission) {
  return permission === "local-network-access";
}

function ipcAllowed(event) {
  const url = ipcFrameUrl(event);
  return Boolean(url) && isAllowedRendererUrl(url);
}

function lockWebContents(contents) {
  contents.on("will-navigate", (e, url) => {
    if (!isAllowedRendererUrl(url)) e.preventDefault();
  });
  contents.on("will-redirect", (e, url) => {
    if (!isAllowedRendererUrl(url)) e.preventDefault();
  });
  contents.on("will-frame-navigate", (e, url) => {
    const target = typeof url === "string" ? url : e && e.url;
    if (!target || !isAllowedRendererUrl(target)) e.preventDefault();
  });
  contents.on("will-attach-webview", (e) => e.preventDefault());
  contents.setWindowOpenHandler(({ url }) => {
    if (isAllowedExternal(url)) {
      void shell.openExternal(url);
    }
    return { action: "deny" };
  });
}

function installAppMenu() {
  const isMac = process.platform === "darwin";
  const viewMenu = app.isPackaged
    ? {
        label: "View",
        submenu: [{ role: "reload" }, { role: "togglefullscreen" }],
      }
    : { role: "viewMenu" };
  const template = [
    ...(isMac ? [{ role: "appMenu" }] : [{ label: "File", submenu: [{ role: "quit" }] }]),
    { role: "editMenu" },
    viewMenu,
    { role: "windowMenu" },
    {
      label: "Help",
      submenu: [
        {
          label: "Check for updates",
          click: () => {
            const win = BrowserWindow.getAllWindows()[0];
            if (win && !win.isDestroyed()) win.webContents.send("late:update-menu-check");
          },
        },
      ],
    },
  ];
  Menu.setApplicationMenu(Menu.buildFromTemplate(template));
}

function createWindow() {
  const win = new BrowserWindow({
    width: 1480,
    height: 940,
    minWidth: 960,
    minHeight: 640,
    title: "Late — Local AI Terminal Emulator",
    backgroundColor: "#0b1020",
    autoHideMenuBar: true,
    icon: path.join(__dirname, "../src-tauri/icons/128x128.png"),
    webPreferences: {
      preload: path.join(__dirname, "preload.cjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      spellcheck: false,
      webviewTag: false,
      devTools: !app.isPackaged,
      webgl: false,
      backgroundThrottling: false,
    },
  });
  if (app.isPackaged) {
    win.loadFile(path.join(__dirname, "../dist/index.html"));
  } else {
    win.loadURL(DEV_URL);
  }
  win.webContents.on("render-process-gone", (_e, details) => {
    console.error("late renderer gone", details);
    win.reload();
  });
  win.webContents.on("gpu-crashed", () => {
    console.error("late gpu crashed");
    win.reload();
  });
}

app.whenReady().then(async () => {
  session.defaultSession.setPermissionRequestHandler((_wc, permission, callback) => {
    callback(isAllowedWebPermission(permission));
  });
  session.defaultSession.setPermissionCheckHandler((_wc, permission) => isAllowedWebPermission(permission));
  ipcMain.handle("late:token", (event) => {
    if (!ipcAllowed(event)) return "";
    return readLocalToken();
  });
  ipcMain.handle("late:clipboard-read", (event) => {
    if (!ipcAllowed(event)) return "";
    try {
      return clipboard.readText().slice(0, 2_000_000);
    } catch {
      return "";
    }
  });
  ipcMain.handle("late:clipboard-write", (event, text, which) => {
    if (!ipcAllowed(event)) return false;
    if (typeof text !== "string" || text.length > 2_000_000) return false;
    const dest = which === "selection" ? "selection" : "clipboard";
    try {
      clipboard.writeText(text, dest);
      return true;
    } catch {
      return false;
    }
  });
  ipcMain.handle("late:is-directory", (event, p) => {
    if (!ipcAllowed(event) || typeof p !== "string" || !p.trim()) return false;
    try {
      return fs.statSync(p).isDirectory();
    } catch {
      return false;
    }
  });
  const update = require("./update.cjs");
  ipcMain.handle("late:update-meta", (event) => {
    if (!ipcAllowed(event)) return null;
    return update.updateMeta();
  });
  ipcMain.handle("late:update-check", async (event, mcpCwd) => {
    if (!ipcAllowed(event)) return { lateError: "blocked", orchError: "blocked" };
    return update.checkUpdates(typeof mcpCwd === "string" ? mcpCwd : "");
  });
  ipcMain.handle("late:update-apply", async (event, payload) => {
    if (!ipcAllowed(event)) return { ok: false, steps: [], error: "blocked" };
    return update.applyUpdates(payload && typeof payload === "object" ? payload : {});
  });
  installAppMenu();
  await startBackend();
  createWindow();
});
app.on("web-contents-created", (_e, contents) => {
  lockWebContents(contents);
});
app.on("window-all-closed", () => {
  stopBackend();
  app.quit();
});
app.on("before-quit", stopBackend);
