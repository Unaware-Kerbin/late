/**
 * Thin Late wrapper around electron/update-sync.cjs (implementation-boss I/O).
 * Compare / JSON shape: apps/desktop/src/lib/updateSync.ts (copy of orchestrator src/update-sync.ts).
 */
const path = require("node:path");
const { app } = require("electron");
const updateSync = require("./update-sync.cjs");

function repoRoot() {
  return path.join(__dirname, "..", "..", "..");
}

function packagedProductVersion() {
  if (!app.isPackaged) return "";
  return typeof app.getVersion === "function" ? app.getVersion() : "";
}

function latePackageVersion() {
  return updateSync.gatherContext(repoRoot(), app.isPackaged, "", packagedProductVersion()).lateLocal;
}

function updateMeta() {
  return {
    lateVersion: latePackageVersion(),
    packaged: app.isPackaged,
    platform: process.platform,
    arch: process.arch,
  };
}

async function checkUpdates(mcpCwd) {
  return updateSync.fetchBoth(
    repoRoot(),
    app.isPackaged,
    typeof mcpCwd === "string" ? mcpCwd : "",
    packagedProductVersion(),
  );
}

async function applyUpdates(payload) {
  if (!payload || payload.confirmed !== true) {
    return { ok: false, steps: [], error: "Nothing was installed. Confirm first." };
  }
  return updateSync.applyConfirmed({
    confirmed: true,
    which: payload.which,
    repoRoot: repoRoot(),
    isPackaged: app.isPackaged,
    packagedVersion: packagedProductVersion(),
    mcpCwd: typeof payload.mcpCwd === "string" ? payload.mcpCwd : "",
  });
}

module.exports = { checkUpdates, applyUpdates, updateMeta };
