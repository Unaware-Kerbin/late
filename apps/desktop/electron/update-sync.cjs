"use strict";

const fs = require("node:fs");
const http = require("node:https");
const os = require("node:os");
const path = require("node:path");

const REPOS = {
  late: "Unaware-Kerbin/late",
  orchestrator: "Unaware-Kerbin/agent-orchestrator",
};
const MAX_ASSET_BYTES = 400 * 1024 * 1024;
const MAX_RELEASE_JSON = 1_500_000;
const GITHUB_DOWNLOAD_PATH = /^\/Unaware-Kerbin\/(late|agent-orchestrator)\/releases\/download\//;
const GITHUB_API_PATH = /^\/repos\/Unaware-Kerbin\/(late|agent-orchestrator)\/releases\/latest$/;

function allowedDownloadHost(url) {
  try {
    const u = new URL(url);
    if (u.protocol !== "https:") return false;
    if (u.username || u.password) return false;
    const host = u.hostname.toLowerCase();
    if (host === "api.github.com") return GITHUB_API_PATH.test(u.pathname);
    if (host === "github.com") return GITHUB_DOWNLOAD_PATH.test(u.pathname);
    return host === "objects.githubusercontent.com" || host === "release-assets.githubusercontent.com";
  } catch {
    return false;
  }
}

function safeAssetFileName(name) {
  const base = path.basename(String(name).replaceAll("\\", "/"));
  if (!base || base === "." || base === ".." || base.includes("\0")) {
    throw new Error("Release file name is not safe.");
  }
  return base;
}

function parseSemver(tag) {
  const m = String(tag || "")
    .trim()
    .replace(/^v/i, "")
    .match(/^(\d+)\.(\d+)\.(\d+)\b/);
  if (!m) return null;
  return { major: Number(m[1]), minor: Number(m[2]), patch: Number(m[3]) };
}

function isNewerRelease(remote, local) {
  const a = parseSemver(remote);
  const b = parseSemver(local);
  if (!a || !b || b.major >= 20) return false;
  if (a.major !== b.major) return a.major > b.major;
  if (a.minor !== b.minor) return a.minor > b.minor;
  return a.patch > b.patch;
}

function usableProductVersion(version) {
  const p = parseSemver(version);
  if (!p || p.major >= 20) return "";
  return `${p.major}.${p.minor}.${p.patch}`;
}

function readPackageVersion(root, fallback) {
  try {
    const raw = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
    return typeof raw.version === "string" && raw.version.trim() ? raw.version.trim() : fallback;
  } catch {
    return fallback;
  }
}

function cacheDir() {
  const home = os.homedir();
  const base =
    process.platform === "win32"
      ? path.join(process.env.LOCALAPPDATA || path.join(home, "AppData", "Local"), "late", "updates")
      : process.platform === "darwin"
        ? path.join(home, "Library", "Caches", "late", "updates")
        : path.join(process.env.XDG_CACHE_HOME || path.join(home, ".cache"), "late", "updates");
  fs.mkdirSync(base, { recursive: true, mode: 0o700 });
  return base;
}

function discoverLateInstall(repoRoot, isPackaged) {
  const appImage = process.env.APPIMAGE;
  if (appImage && fs.existsSync(appImage)) return { kind: "appimage", path: appImage };
  if (isPackaged) {
    if (process.platform === "linux") return { kind: "deb", path: process.resourcesPath };
    if (process.platform === "darwin") return { kind: "dmg", path: process.resourcesPath };
    return { kind: "nsis", path: process.resourcesPath };
  }
  return { kind: "git-npm", path: repoRoot };
}

function discoverOrchInstall(mcpCwd, repoRoot) {
  const candidates = [
    process.env.UPDATE_SYNC_ORCH_ROOT,
    mcpCwd,
    path.join(repoRoot, "..", "MCP_Server_AI_Agent_Project"),
    path.join(os.homedir(), "Documents", "MCP_Server_AI_Agent_Project"),
  ].filter((p) => typeof p === "string" && p.trim());
  for (const raw of candidates) {
    const root = path.resolve(raw.trim());
    if (!fs.existsSync(path.join(root, "package.json"))) continue;
    try {
      const pkg = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
      if (pkg.name !== "agent-orchestrator") continue;
    } catch {
      continue;
    }
    const portable =
      fs.existsSync(path.join(root, "runtime", "bin", "node")) || fs.existsSync(path.join(root, "runtime", "node.exe"));
    return { kind: portable ? "portable" : "git-npm", path: root };
  }
  return { kind: "unknown", path: "" };
}

function latePrefer(install) {
  if (install.kind === "deb") return "deb";
  if (install.kind === "appimage") return "appimage";
  return "auto";
}

function githubGet(url) {
  if (
    url !== `https://api.github.com/repos/${REPOS.late}/releases/latest` &&
    url !== `https://api.github.com/repos/${REPOS.orchestrator}/releases/latest`
  ) {
    return Promise.resolve({ status: 0, body: null, error: "Update check only reads Unaware-Kerbin latest." });
  }
  return new Promise((resolve) => {
    let settled = false;
    const finish = (value) => {
      if (settled) return;
      settled = true;
      resolve(value);
    };
    const req = http.get(
      url,
      {
        headers: {
          "User-Agent": "Late-update-sync",
          Accept: "application/vnd.github+json",
          "X-GitHub-Api-Version": "2022-11-28",
          // Never Authorization / GITHUB_TOKEN / CURSOR_API_KEY / MCP tokens.
        },
      },
      (res) => {
        const chunks = [];
        let size = 0;
        res.on("data", (c) => {
          size += c.length;
          if (size > MAX_RELEASE_JSON) {
            req.destroy();
            finish({ status: 0, body: null, error: "GitHub reply was larger than I will read." });
            return;
          }
          chunks.push(c);
        });
        res.on("end", () => {
          if (settled) return;
          const text = Buffer.concat(chunks).toString("utf8");
          let body = null;
          try {
            body = text ? JSON.parse(text) : null;
          } catch {
            body = null;
          }
          finish({ status: res.statusCode || 0, body });
        });
      },
    );
    req.on("error", (err) => finish({ status: 0, body: null, error: err.message }));
    req.setTimeout(20000, () => {
      req.destroy();
      finish({ status: 0, body: null, error: "GitHub request timed out" });
    });
  });
}

function downloadTo(url, dest, redirects = 5) {
  return new Promise((resolve, reject) => {
    let first;
    try {
      first = new URL(url);
    } catch (err) {
      reject(err);
      return;
    }
    if (first.hostname.toLowerCase() !== "github.com" || !allowedDownloadHost(url)) {
      reject(new Error("Download must start at github.com/Unaware-Kerbin/…/releases/download/."));
      return;
    }
    const file = fs.createWriteStream(dest, { mode: 0o600 });
    let size = 0;
    const fail = (err) => {
      try {
        file.destroy();
        fs.unlinkSync(dest);
      } catch {
        /* ignore */
      }
      reject(err instanceof Error ? err : new Error(String(err)));
    };
    const go = (current, left) => {
      if (!allowedDownloadHost(current)) {
        fail(new Error("Refusing redirect off GitHub https."));
        return;
      }
      const req = http.get(
        current,
        { headers: { "User-Agent": "Late-update-sync", Accept: "application/octet-stream" } },
        (res) => {
          const loc = res.headers.location;
          if (res.statusCode >= 300 && res.statusCode < 400 && loc && left > 0) {
            res.resume();
            let next;
            try {
              next = new URL(loc, current).href;
            } catch {
              fail(new Error("GitHub redirect had no next address."));
              return;
            }
            go(next, left - 1);
            return;
          }
          if (res.statusCode !== 200) {
            res.resume();
            fail(new Error(`Download failed HTTP ${res.statusCode}`));
            return;
          }
          res.on("data", (chunk) => {
            size += chunk.length;
            if (size > MAX_ASSET_BYTES) {
              res.destroy();
              fail(new Error("That file is larger than I will save on your computer."));
            }
          });
          res.pipe(file);
          file.on("finish", () => file.close(() => resolve()));
          file.on("error", fail);
        },
      );
      req.on("error", fail);
      req.setTimeout(120000, () => {
        req.destroy();
        fail(new Error("Download timed out"));
      });
    };
    go(url, redirects);
  });
}

function pickAsset(app, release, platform, arch, prefer) {
  const list = Array.isArray(release?.assets) ? release.assets : [];
  const osNeed = platform === "darwin" ? ["mac", "darwin"] : platform === "win32" ? ["win"] : ["linux"];
  const archNeed = arch === "arm64" ? ["arm64", "aarch64"] : ["x64", "x86_64", "amd64"];
  let best = null;
  let bestScore = 0;
  for (const a of list) {
    if (!a || typeof a.name !== "string" || typeof a.browser_download_url !== "string") continue;
    const n = a.name.toLowerCase();
    if (!osNeed.some((s) => n.includes(s))) continue;
    if (!archNeed.some((s) => n.includes(s))) continue;
    if (arch !== "arm64" && /arm64|aarch64/.test(n)) continue;
    let ext = 0;
    if (app === "late") {
      if (platform === "linux") {
        if (prefer === "deb") ext = n.endsWith(".deb") ? 4 : n.endsWith(".appimage") ? 1 : 0;
        else ext = n.endsWith(".appimage") ? 4 : n.endsWith(".deb") ? 1 : 0;
      } else if (platform === "darwin") ext = n.endsWith(".dmg") ? 4 : n.endsWith(".zip") ? 2 : 0;
      else ext = n.endsWith(".exe") ? 4 : n.endsWith(".msi") ? 2 : 0;
    } else {
      ext =
        platform === "win32"
          ? n.endsWith(".zip")
            ? 4
            : 0
          : n.endsWith(".tar.gz") || n.endsWith(".tgz")
            ? 4
            : 0;
    }
    if (ext > bestScore) {
      best = a;
      bestScore = ext;
    }
  }
  if (!best) return null;
  return { name: best.name, url: best.browser_download_url };
}

async function applyOne(app, asset) {
  const dest = path.join(cacheDir(), safeAssetFileName(asset.name));
  await downloadTo(asset.url, dest);
  if (app === "late" && dest.toLowerCase().endsWith(".appimage")) {
    try {
      fs.chmodSync(dest, 0o755);
    } catch {
      /* ignore */
    }
  }
  const unsigned =
    process.platform === "darwin" || process.platform === "win32"
      ? " macOS and Windows installers are unsigned — right-click Open / More info → Run anyway."
      : "";
  if (app === "late") {
    return {
      app,
      ok: true,
      dest,
      message: `Saved ${asset.name} on your computer (${dest}). I did not run the installer or use root.${unsigned}`,
    };
  }
  return {
    app,
    ok: true,
    dest,
    message: `Saved ${asset.name} on your computer (${dest}). I did not check out a git tag, overwrite the running copy, or run anything.${unsigned}`,
  };
}

function gatherContext(repoRoot, isPackaged, mcpCwd) {
  const lateInstall = discoverLateInstall(repoRoot, isPackaged);
  const orchInstall = discoverOrchInstall(mcpCwd, repoRoot);
  const lateLocal =
    usableProductVersion(process.env.UPDATE_SYNC_LATE_LOCAL) ||
    usableProductVersion(readPackageVersion(path.join(repoRoot, "apps", "desktop"), "")) ||
    usableProductVersion(readPackageVersion(repoRoot, ""));
  const orchLocal =
    usableProductVersion(process.env.UPDATE_SYNC_ORCH_LOCAL) ||
    usableProductVersion(orchInstall.path ? readPackageVersion(orchInstall.path, "") : "");
  return {
    lateInstall,
    orchInstall,
    lateLocal,
    orchLocal,
    prefer: latePrefer(lateInstall),
    platform: process.platform === "darwin" || process.platform === "win32" ? process.platform : "linux",
    arch: process.arch === "arm64" ? "arm64" : "x64",
  };
}

async function fetchRelease(app) {
  const url = `https://api.github.com/repos/${REPOS[app]}/releases/latest`;
  const res = await githubGet(url);
  if (res.status === 404) return { ok: false, status: 404, error: "No GitHub release found (404)." };
  if (res.status !== 200 || !res.body) {
    return { ok: false, status: res.status || 0, error: res.error || `GitHub returned HTTP ${res.status || 0}.` };
  }
  return { ok: true, status: res.status, release: res.body };
}

async function fetchBoth(repoRoot, isPackaged, mcpCwd) {
  const ctx = gatherContext(repoRoot, isPackaged, mcpCwd);
  const [late, orch] = await Promise.all([fetchRelease("late"), fetchRelease("orchestrator")]);
  return {
    ...ctx,
    lateRelease: late.release || null,
    lateError: late.error || null,
    orchRelease: orch.release || null,
    orchError: orch.error || null,
  };
}

async function applyConfirmed(opts) {
  if (!opts || opts.confirmed !== true) {
    return { ok: false, steps: [], error: "Nothing was installed. Confirm first." };
  }
  const which = opts.which;
  const targets = which === "both" ? ["late", "orchestrator"] : which === "late" || which === "orchestrator" ? [which] : [];
  if (!targets.length) return { ok: false, steps: [], error: "Choose Late, Orchestrator, or both." };
  const both = await fetchBoth(opts.repoRoot, opts.isPackaged, opts.mcpCwd);
  const steps = [];
  for (const app of targets) {
    const release = app === "late" ? both.lateRelease : both.orchRelease;
    const fetchError = app === "late" ? both.lateError : both.orchError;
    const local = app === "late" ? both.lateLocal : both.orchLocal;
    if (fetchError || !release) {
      steps.push({
        app,
        ok: false,
        message: fetchError || "GitHub did not return a release. Nothing was downloaded.",
      });
      continue;
    }
    if (!usableProductVersion(local)) {
      steps.push({
        app,
        ok: false,
        message: "Could not read this app's version on your computer. I will not skip or force a download.",
      });
      continue;
    }
    if (!isNewerRelease(release?.tag_name, local)) {
      steps.push({
        app,
        ok: true,
        message: "That copy on your computer is already current. Nothing was downloaded.",
      });
      continue;
    }
    const asset = pickAsset(app, release, both.platform, both.arch, both.prefer);
    if (!asset) {
      steps.push({
        app,
        ok: false,
        message: "GitHub has no matching file for this computer. I will not build from the tag.",
      });
      continue;
    }
    if (!allowedDownloadHost(asset.url)) {
      steps.push({ app, ok: false, message: "Refusing download: URL is not GitHub https." });
      continue;
    }
    try {
      steps.push(await applyOne(app, asset));
    } catch (err) {
      steps.push({ app, ok: false, message: err instanceof Error ? err.message : String(err) });
    }
  }
  return { ok: steps.every((s) => s.ok), steps };
}

module.exports = {
  fetchBoth,
  applyConfirmed,
  gatherContext,
  allowedDownloadHost,
  usableProductVersion,
};
