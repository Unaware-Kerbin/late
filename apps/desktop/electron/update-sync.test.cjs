"use strict";

const assert = require("node:assert/strict");
const path = require("node:path");
const test = require("node:test");
const update = require("./update-sync.cjs");

test("Late CJS host allowlist refuses SSRF and custom GitHub URLs", () => {
  assert.equal(
    update.allowedDownloadHost(
      "https://github.com/Unaware-Kerbin/late/releases/download/v0.1.9/Late-0.1.9-linux-x64.AppImage",
    ),
    true,
  );
  assert.equal(update.allowedDownloadHost("https://raw.githubusercontent.com/evil/malware/main/pwn"), false);
  assert.equal(update.allowedDownloadHost("https://github.com/evil/late/releases/download/v1/x.bin"), false);
  assert.equal(update.allowedDownloadHost("https://127.0.0.1/secret"), false);
  assert.equal(
    update.allowedDownloadHost("https://user:token@github.com/Unaware-Kerbin/late/releases/download/v0.1.9/x.bin"),
    false,
  );
});

test("gatherContext lateLocal is package.json, not Electron 44", () => {
  const root = path.join(__dirname, "..", "..", "..");
  const ctx = update.gatherContext(root, false, "");
  assert.equal(ctx.lateLocal, "0.1.9");
  assert.notEqual(ctx.lateLocal, "44.0.0");
});

test("usableProductVersion rejects Electron 44 and env override cannot skip", () => {
  assert.equal(update.usableProductVersion("44.0.0"), "");
  assert.equal(update.usableProductVersion("28.3.1"), "");
  assert.equal(update.usableProductVersion("0.1.9"), "0.1.9");
  const prev = process.env.UPDATE_SYNC_LATE_LOCAL;
  process.env.UPDATE_SYNC_LATE_LOCAL = "44.0.0";
  try {
    const root = path.join(__dirname, "..", "..", "..");
    const ctx = update.gatherContext(root, false, "");
    assert.equal(ctx.lateLocal, "0.1.9");
  } finally {
    if (prev === undefined) delete process.env.UPDATE_SYNC_LATE_LOCAL;
    else process.env.UPDATE_SYNC_LATE_LOCAL = prev;
  }
});

test("applyConfirmed refuses without confirm", async () => {
  const denied = await update.applyConfirmed({ which: "both", confirmed: false });
  assert.equal(denied.ok, false);
  assert.match(denied.error ?? "", /Confirm first/);
});
