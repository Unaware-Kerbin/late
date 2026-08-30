"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const update = require("./update-sync.cjs");

const WRONG_DIGEST = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

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

test("CDN redirect off GitHub is refused", () => {
  assert.equal(
    update.allowedCdnRedirectUrl("https://evil.example/github-production-release-asset/x", "Late.AppImage", true),
    false,
  );
  assert.equal(update.allowedFirstDownloadUrl("https://github.com/evil/late/releases/download/v1/x.bin", "x.bin"), false);
  assert.equal(
    update.allowedDownloadHost("https://objects.githubusercontent.com/not-a-release-asset/x", "Late.AppImage", {
      hop: "cdn",
      digestPinned: true,
    }),
    false,
  );
});

test("malformed digest is not skipped", () => {
  assert.throws(() => update.parseAssetDigest("md5:abc"), /SHA-256/);
  assert.equal(
    update.parseAssetDigest("sha256:277089d91c0bdf4f2e6862ba7e4a07605119431f5d13f726dd352b06f1b206a9"),
    "277089d91c0bdf4f2e6862ba7e4a07605119431f5d13f726dd352b06f1b206a9",
  );
});

test("verifyFileDigest fails closed on mismatch and deletes the file", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "late-digest-"));
  const dest = path.join(dir, "Late-0.2.0-linux-x64.AppImage");
  fs.writeFileSync(dest, "bytes");
  await assert.rejects(() => update.verifyFileDigest(dest, WRONG_DIGEST), /did not match/);
  assert.equal(fs.existsSync(dest), false);
  fs.rmSync(dir, { recursive: true, force: true });
});

test("CJS CDN redirect is pinned to the chosen asset", () => {
  const file = "Late-0.1.9-linux-x64.AppImage";
  const first = `https://github.com/Unaware-Kerbin/late/releases/download/v0.1.9/${file}`;
  assert.equal(update.allowedFirstDownloadUrl(first, file), true);
  assert.equal(update.allowedFirstDownloadUrl(first, "other.bin"), false);
  const cdn = `https://release-assets.githubusercontent.com/github-production-release-asset/1/2?rscd=attachment%3B%20filename%3D${file}`;
  assert.equal(update.allowedCdnRedirectUrl(cdn, file, false), true);
  assert.equal(update.allowedCdnRedirectUrl(cdn, "other.bin", true), false);
  assert.equal(update.allowedDownloadHost("https://objects.githubusercontent.com/evil/pwn"), false);
  assert.equal(update.parseAssetDigest("sha256:277089d91c0bdf4f2e6862ba7e4a07605119431f5d13f726dd352b06f1b206a9"), "277089d91c0bdf4f2e6862ba7e4a07605119431f5d13f726dd352b06f1b206a9");
  assert.throws(() => update.parseAssetDigest("md5:abc"), /SHA-256/);
  assert.equal(
    update.sha256HexMatches(
      "277089d91c0bdf4f2e6862ba7e4a07605119431f5d13f726dd352b06f1b206a9",
      "0000000000000000000000000000000000000000000000000000000000000000",
    ),
    false,
  );
});
