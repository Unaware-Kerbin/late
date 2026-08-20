const { app, BrowserWindow, shell, ipcMain } = require("electron");
const { spawn } = require("node:child_process");
const http = require("node:http");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

app.disableHardwareAcceleration();
app.commandLine.appendSwitch("disable-gpu");
app.commandLine.appendSwitch("disable-gpu-compositing");
app.commandLine.appendSwitch("disable-gpu-vsync");
app.commandLine.appendSwitch("log-level", "3");
if (process.platform === "linux") {
  app.commandLine.appendSwitch("ozone-platform-hint", "x11");
}

const DEV_URL = process.env.LATE_DEV_URL || "http://127.0.0.1:5173";
const children = [];

function resource(...parts) {
  if (app.isPackaged) return path.join(process.resourcesPath, ...parts);
  return path.join(__dirname, "..", "resources", ...parts);
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

function startChild(bin, args, extraEnv) {
  if (!fs.existsSync(bin)) {
    console.warn("late: missing", bin);
    return null;
  }
  const child = spawn(bin, args, {
    env: { ...process.env, ...extraEnv },
    stdio: "ignore",
    windowsHide: true,
  });
  child.on("error", (err) => console.error("late child error", bin, err));
  children.push(child);
  return child;
}

async function startBackend() {
  if (!app.isPackaged) return;
  const daemonName = process.platform === "win32" ? "late-daemon.exe" : "late-daemon";
  const daemonBin = resource("bin", daemonName);
  const sidecarJs = resource("sidecar.cjs");
  const policies = resource("policies");
  startChild(daemonBin, ["--bind", "127.0.0.1:7420"], {
    LATE_POLICIES_DIR: policies,
  });
  await waitHttp("http://127.0.0.1:7420/health").catch((err) => {
    console.error("late: daemon did not come up", err);
  });
  startChild(process.execPath, [sidecarJs], {
    ELECTRON_RUN_AS_NODE: "1",
    LATE_SIDECAR_PORT: "7430",
    LATE_DAEMON_HTTP: "http://127.0.0.1:7420",
    LATE_DAEMON_WS: "ws://127.0.0.1:7420/ws",
    LATE_RPC_TOKEN: readLocalToken(),
  });
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

function isAllowedRendererUrl(url) {
  try {
    const u = new URL(url);
    if (!app.isPackaged) {
      return u.origin === new URL(DEV_URL).origin;
    }
    if (u.protocol !== "file:") return false;
    const filePath = path.normalize(decodeURIComponent(u.pathname));
    return (
      filePath.endsWith(`${path.sep}dist${path.sep}index.html`) && !filePath.includes("..")
    );
  } catch {
    return false;
  }
}

function isAllowedExternal(url) {
  try {
    const u = new URL(url);
    return u.protocol === "https:" || u.protocol === "http:";
  } catch {
    return false;
  }
}

function lockWebContents(contents) {
  contents.on("will-navigate", (e, url) => {
    if (!isAllowedRendererUrl(url)) e.preventDefault();
  });
  contents.on("will-redirect", (e, url) => {
    if (!isAllowedRendererUrl(url)) e.preventDefault();
  });
  contents.setWindowOpenHandler(({ url }) => {
    if (isAllowedExternal(url)) {
      shell.openExternal(url);
    }
    return { action: "deny" };
  });
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
      webgl: false,
      backgroundThrottling: false,
    },
  });
  win.removeMenu();
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
  ipcMain.handle("late:token", (event) => {
    const url = event.senderFrame?.url || event.sender.getURL();
    if (!isAllowedRendererUrl(url)) return "";
    return readLocalToken();
  });
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
