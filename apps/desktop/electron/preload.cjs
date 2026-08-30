const { contextBridge, ipcRenderer, webUtils } = require("electron");

contextBridge.exposeInMainWorld("lateRuntime", {
  token: () => ipcRenderer.invoke("late:token"),
  clipboardRead: () => ipcRenderer.invoke("late:clipboard-read"),
  clipboardWrite: (text, which) => ipcRenderer.invoke("late:clipboard-write", text, which),
  pathForFile: (file) => {
    try {
      return webUtils.getPathForFile(file) || "";
    } catch {
      return "";
    }
  },
  updateMeta: () => ipcRenderer.invoke("late:update-meta"),
  checkUpdates: (mcpCwd) => ipcRenderer.invoke("late:update-check", typeof mcpCwd === "string" ? mcpCwd : ""),
  applyUpdates: (payload) => ipcRenderer.invoke("late:update-apply", payload),
  onUpdateStartup: (cb) => {
    const handler = (_e, payload) => cb(payload);
    ipcRenderer.on("late:update-startup", handler);
    return () => ipcRenderer.removeListener("late:update-startup", handler);
  },
  onUpdateMenuCheck: (cb) => {
    const handler = () => cb();
    ipcRenderer.on("late:update-menu-check", handler);
    return () => ipcRenderer.removeListener("late:update-menu-check", handler);
  },
  isDirectory: (p) => {
    if (typeof p !== "string" || !p.trim()) return Promise.resolve(false);
    if (typeof webUtils.isDirectory === "function") {
      try {
        return Promise.resolve(webUtils.isDirectory(p)).then(
          (v) => v === true,
          () => false,
        );
      } catch {
        return Promise.resolve(false);
      }
    }
    return ipcRenderer.invoke("late:is-directory", p).then(
      (v) => v === true,
      () => false,
    );
  },
});
