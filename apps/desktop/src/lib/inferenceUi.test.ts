import assert from "node:assert/strict";
import { readFileSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { vllmDockerHint, vllmStartVisible } from "./inferenceUi.ts";

test("vLLM Start stays hidden without Docker", () => {
  assert.equal(
    vllmStartVisible({ allowIntelCompose: true, dockerAvailable: false, networkServer: false }),
    false,
  );
  assert.equal(
    vllmStartVisible({ allowIntelCompose: true, dockerAvailable: true, networkServer: false }),
    true,
  );
  assert.equal(
    vllmStartVisible({ allowIntelCompose: false, dockerAvailable: true, networkServer: false }),
    false,
  );
  assert.equal(
    vllmStartVisible({ allowIntelCompose: true, dockerAvailable: true, networkServer: true }),
    false,
  );
});

test("missing Docker shows the install note", () => {
  const hint = vllmDockerHint({
    dockerAvailable: false,
    allowIntelCompose: true,
    networkServer: false,
  });
  assert.match(hint ?? "", /Install Docker to use vLLM/);
  assert.equal(
    vllmDockerHint({ dockerAvailable: true, allowIntelCompose: true, networkServer: false }),
    null,
  );
});

test("pack lists bundled bin, lib, docker compose extraResources", () => {
  const here = dirname(fileURLToPath(import.meta.url));
  const yml = readFileSync(join(here, "..", "..", "electron-builder.yml"), "utf8");
  assert.match(yml, /from: resources\/bin/);
  assert.match(yml, /from: resources\/lib/);
  assert.match(yml, /from: resources\/docker/);
  assert.match(yml, /from: resources-mac\/bin/);
  assert.match(yml, /from: resources-mac\/lib/);
  assert.match(yml, /from: resources-mac\/docker/);
  assert.match(yml, /from: resources-win\/bin/);
  assert.match(yml, /from: resources-win\/lib/);
  assert.match(yml, /from: resources-win\/docker/);
  assert.match(yml, /--deb-suggests=docker\.io/);
  assert.match(yml, /--deb-templates=\.\.\/\.\.\/installer\/linux\/templates\s*$/m);
  assert.doesNotMatch(yml, /--deb-templates=.*\/$/m);
  assert.doesNotMatch(yml, /--deb-recommends=/);
  assert.match(yml, /^\s+- rpm\s*$/m);
  assert.match(yml, /^\s+- pacman\s*$/m);
  assert.match(yml, /--pacman-optional-depends/);
  assert.match(yml, /--rpm-tag=Suggests: docker/);
  assert.doesNotMatch(yml, /--rpm-tag=Requires:/);
  assert.doesNotMatch(yml, /--deb-depends=docker/);
  const templatesPath = join(here, "..", "..", "..", "..", "installer", "linux", "templates");
  assert.equal(statSync(templatesPath).isFile(), true);
  assert.match(readFileSync(templatesPath, "utf8"), /Template:\s*late\/install-docker/);
  assert.match(readFileSync(templatesPath, "utf8"), /Default:\s*false/);
  const pack = readFileSync(join(here, "..", "..", "..", "..", "scripts", "pack.sh"), "utf8");
  assert.match(pack, /fetch-inference-bins\.sh/);
  assert.match(pack, /docker\/compose\.yml/);
  assert.match(pack, /resources-mac/);
  assert.match(pack, /resources-win/);
  assert.match(pack, /INFERENCE_TARGET/);
  assert.match(pack, /RES_DIR=/);
  assert.match(pack, /--win/);
  assert.match(pack, /--mac/);
  assert.match(pack, /cargo release daemon \(windows\)/);
  assert.match(pack, /cargo release daemon \(mac\)/);
  assert.match(pack, /reuse existing inference bins/);
  assert.match(pack, /PACK_LINUX_ALL/);
  assert.match(pack, /--linux rpm pacman/);
});

test("fetch-inference-bins.sh pins find/curl/sha256sum off PATH", () => {
  const here = dirname(fileURLToPath(import.meta.url));
  const src = readFileSync(join(here, "..", "..", "..", "..", "scripts", "fetch-inference-bins.sh"), "utf8");
  assert.match(src, /FIND="\$\(secure_bin find\)"/);
  assert.match(src, /CURL="\$\(secure_bin curl\)"/);
  assert.match(src, /SHA256SUM="\$\(secure_bin sha256sum\)"/);
  assert.match(src, /\$FIND/);
  assert.match(src, /\$CURL/);
  assert.match(src, /\$SHA256SUM/);
  assert.match(src, /\$\{expect\}-\$\("\$BASENAME" "\$file"\)/);
  assert.match(src, /SHA-256 mismatch/);
  assert.match(src, /INFERENCE_TARGET/);
  assert.match(src, /darwin-arm64/);
  const code = src
    .split("\n")
    .filter((line) => !line.trim().startsWith("#"))
    .join("\n");
  assert.doesNotMatch(code, /(?:^|[\n;|&])\s*find\s+"/m);
  assert.doesNotMatch(code, /(?:^|[\n;|&])\s*curl\s+-/m);
  assert.doesNotMatch(code, /(?:^|[\n;|&])\s*sha256sum\s+"/m);
  assert.match(src, /win-x64/);
  assert.match(src, /mac-arm64/);
  assert.match(src, /mac-x64/);
  assert.match(src, /mlx_metal\*/);
  assert.match(src, /copy_engine_dir "\$ollama_bin" "\$DEST\/bin" "\$DEST\/lib\/ollama"/);
  assert.match(src, /copy_engine_dir "\$llama_bin" "\$DEST\/bin"/);
  const strip = src.slice(src.indexOf("strip_ollama_gpu_libs"), src.indexOf("\nwork="));
  assert.match(strip, /cuda\*/);
  assert.match(strip, /rocm\*/);
  assert.doesNotMatch(strip, /mlx\*/);
});
