const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("lateRuntime", {
  token: () => ipcRenderer.invoke("late:token"),
});
