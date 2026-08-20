const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("lateRuntime", {
  token: () => ipcRenderer.invoke("late:token"),
  clipboardRead: () => ipcRenderer.invoke("late:clipboard-read"),
  clipboardWrite: (text, which) => ipcRenderer.invoke("late:clipboard-write", text, which),
});
