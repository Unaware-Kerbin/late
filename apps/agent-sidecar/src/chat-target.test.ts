import assert from "node:assert/strict";
import test from "node:test";
import {
  classifyChatBaseSync,
  classifyHostname,
  parseExtraHosts,
} from "./chat-target.ts";

test("loopback names and IPs", () => {
  assert.equal(classifyHostname("127.0.0.1"), "loopback");
  assert.equal(classifyHostname("localhost"), "loopback");
  assert.equal(classifyHostname("::1"), "loopback");
  assert.equal(classifyHostname("[::1]"), "loopback");
  assert.equal(classifyHostname("127.0.0.2"), "loopback");
});

test("RFC1918, link-local, ULA", () => {
  assert.equal(classifyHostname("10.1.2.3"), "private");
  assert.equal(classifyHostname("192.168.0.5"), "private");
  assert.equal(classifyHostname("172.16.9.1"), "private");
  assert.equal(classifyHostname("172.31.255.1"), "private");
  assert.equal(classifyHostname("169.254.1.1"), "private");
  assert.equal(classifyHostname("fd12:3456:789a::1"), "private");
  assert.equal(classifyHostname("fe80::1"), "private");
});

test("public IPs stay cloud", () => {
  assert.equal(classifyHostname("8.8.8.8"), "cloud");
  assert.equal(classifyHostname("1.1.1.1"), "cloud");
  assert.equal(classifyHostname("172.15.0.1"), "cloud");
  assert.equal(classifyHostname("172.32.0.1"), "cloud");
  assert.equal(classifyHostname("100.64.0.1"), "cloud");
});

test("private DNS suffixes", () => {
  assert.equal(classifyHostname("vllm.lab.internal"), "private");
  assert.equal(classifyHostname("gpu.local"), "private");
  assert.equal(classifyHostname("box.lan"), "private");
  assert.equal(classifyHostname("nas.home.arpa"), "private");
  assert.equal(classifyHostname("gpu.lab.internal.example.com"), "cloud");
  assert.equal(classifyHostname("api.openai.com"), "cloud");
});

test("extra host allowlist is exact", () => {
  const extra = parseExtraHosts("llm.airgap.mil, GPU.LAB");
  assert.deepEqual(extra, ["llm.airgap.mil", "gpu.lab"]);
  assert.equal(classifyHostname("llm.airgap.mil", extra), "private");
  assert.equal(classifyHostname("api.airgap.mil", extra), "cloud");
  assert.equal(classifyHostname("api.openai.com", extra), "cloud");
});

test("parseExtraHosts ignores wildcards and junk", () => {
  assert.deepEqual(parseExtraHosts(" * , , foo..bar, ok.host "), ["ok.host"]);
  assert.deepEqual(parseExtraHosts(""), []);
  assert.deepEqual(parseExtraHosts(undefined), []);
});

test("base URLs", () => {
  assert.equal(classifyChatBaseSync("http://127.0.0.1:8000/v1"), "loopback");
  assert.equal(classifyChatBaseSync("http://10.0.0.12:8000/v1"), "private");
  assert.equal(classifyChatBaseSync("https://vllm.internal/v1"), "private");
  assert.equal(classifyChatBaseSync("https://api.openai.com/v1"), "cloud");
  assert.equal(classifyChatBaseSync("ftp://10.0.0.1/v1"), "cloud");
  assert.equal(classifyChatBaseSync("not a url"), "cloud");
});
