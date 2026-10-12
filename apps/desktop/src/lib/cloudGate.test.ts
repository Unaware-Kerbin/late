import assert from "node:assert/strict";
import test from "node:test";
import {
  backendBlockedWhenCloudOff,
  backendNeedsCloudAi,
  classifyChatBaseSync,
  cloudOffOptionSuffix,
} from "./cloudGate.ts";

test("classifyChatBaseSync: loopback and private stay off Cloud AI", () => {
  assert.equal(classifyChatBaseSync("http://127.0.0.1:8000/v1"), "loopback");
  assert.equal(classifyChatBaseSync("http://10.0.0.12:8000/v1"), "private");
  assert.equal(classifyChatBaseSync("http://llm.lab.internal/v1"), "private");
  assert.equal(classifyChatBaseSync("http://gpu.home.arpa/v1"), "private");
  assert.equal(classifyChatBaseSync("http://proxy.airgap.mil/v1", "proxy.airgap.mil"), "private");
});

test("classifyChatBaseSync: public SaaS is cloud", () => {
  assert.equal(classifyChatBaseSync("https://api.anthropic.com"), "cloud");
  assert.equal(classifyChatBaseSync("https://generativelanguage.googleapis.com"), "cloud");
  assert.equal(classifyChatBaseSync("https://myres.openai.azure.com"), "cloud");
  assert.equal(classifyChatBaseSync(""), "cloud");
});

test("backendNeedsCloudAi: Cursor always; public natives yes; private no", () => {
  assert.equal(backendNeedsCloudAi("cursor"), true);
  assert.equal(backendNeedsCloudAi("anthropic", { baseUrl: "https://api.anthropic.com" }), true);
  assert.equal(backendNeedsCloudAi("anthropic", { baseUrl: "http://10.1.2.3:8080" }), false);
  assert.equal(backendNeedsCloudAi("gemini", { baseUrl: "https://generativelanguage.googleapis.com" }), true);
  assert.equal(backendNeedsCloudAi("azure", { baseUrl: "https://x.openai.azure.com" }), true);
  assert.equal(backendNeedsCloudAi("azure", { baseUrl: "http://10.0.0.5" }), false);
  assert.equal(backendNeedsCloudAi("local", { baseUrl: "http://127.0.0.1:8000/v1" }), false);
  assert.equal(backendNeedsCloudAi("mcp", { mcpStdio: true }), false);
  assert.equal(backendNeedsCloudAi("mcp", { baseUrl: "https://mcp.example.com/mcp" }), true);
  assert.equal(backendNeedsCloudAi("mcp", { baseUrl: "http://127.0.0.1:8790/mcp" }), false);
});

test("backendBlockedWhenCloudOff greys public platforms only", () => {
  assert.equal(backendBlockedWhenCloudOff("cursor", true), false);
  assert.equal(backendBlockedWhenCloudOff("cursor", false), true);
  assert.equal(
    backendBlockedWhenCloudOff("anthropic", false, { base: "https://api.anthropic.com", egress: "cloud" }),
    true,
  );
  assert.equal(
    backendBlockedWhenCloudOff("anthropic", false, { base: "http://10.0.0.9", egress: "private" }),
    false,
  );
  assert.equal(
    backendBlockedWhenCloudOff("local", false, { base: "http://127.0.0.1:8000/v1", egress: "loopback" }),
    false,
  );
  assert.equal(cloudOffOptionSuffix(true, false), " — Cloud AI is off");
  assert.equal(cloudOffOptionSuffix(true, true), "");
});
