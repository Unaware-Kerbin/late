import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  isOrchestratorLogoPath,
  isOrchestratorLogoUrl,
  logoUrlCandidates,
} from "./orch-rest.ts";

describe("orchestrator logo URLs", () => {
  it("accepts only /api/backends/<id>/logo on loopback", () => {
    assert.equal(isOrchestratorLogoPath("/api/backends/vllm-local/logo"), true);
    assert.equal(isOrchestratorLogoPath("/api/backends/gemini/logo"), true);
    assert.equal(isOrchestratorLogoPath("/api/chats"), false);
    assert.equal(isOrchestratorLogoPath("/rpc"), false);
    assert.equal(isOrchestratorLogoPath("/api/backends/../secret/logo"), false);
    assert.equal(
      isOrchestratorLogoUrl("http://127.0.0.1:8787/api/backends/vllm-local/logo"),
      true,
    );
    assert.equal(isOrchestratorLogoUrl("http://127.0.0.1:7420/rpc"), false);
    assert.equal(isOrchestratorLogoUrl("http://127.0.0.1:9/api/secret"), false);
    assert.equal(isOrchestratorLogoUrl("http://10.0.0.12:8787/api/backends/vllm-local/logo"), false);
  });

  it("does not send Bearer candidates to non-logo paths", () => {
    const origins = ["http://127.0.0.1:8787"];
    assert.deepEqual(logoUrlCandidates("http://127.0.0.1:7420/rpc", origins), []);
    assert.deepEqual(logoUrlCandidates("/api/chats", origins), []);
    assert.deepEqual(logoUrlCandidates("/pending", origins), []);
    const ok = logoUrlCandidates("http://127.0.0.1:8787/api/backends/vllm-local/logo", origins);
    assert.deepEqual(ok, ["http://127.0.0.1:8787/api/backends/vllm-local/logo"]);
    const rel = logoUrlCandidates("/api/backends/gemini/logo", origins);
    assert.deepEqual(rel, ["http://127.0.0.1:8787/api/backends/gemini/logo"]);
  });
});
