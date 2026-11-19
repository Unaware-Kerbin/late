import assert from "node:assert/strict";
import { test } from "node:test";
import {
  appendSessionPrebuffer,
  encodeTermDataFrame,
  encodeTermInputFrame,
  MAX_SESSION_PREBUFFER_BYTES,
  parseTermDataFrame,
  isCancelled,
  isConnectError,
  RpcError,
  hostKeyEventInfo,
  sessionOpenedInfo,
} from "./rpc";

test("encodeTermInputFrame is sessionId NUL payload, not JSON-RPC", () => {
  const buf = encodeTermInputFrame("a1b2c3d4-e5f6-7890-abcd-ef1234567890", "sh");
  const z = buf.indexOf(0);
  assert.equal(z, 36);
  assert.equal(new TextDecoder().decode(buf.subarray(0, z)), "a1b2c3d4-e5f6-7890-abcd-ef1234567890");
  assert.equal(new TextDecoder().decode(buf.subarray(z + 1)), "sh");
  assert.equal(buf.includes(0x7b), false, "must not be a JSON object");
});

test("parseTermDataFrame round-trips stdout bytes", () => {
  const payload = new TextEncoder().encode("ArubaOS-CX\r\n");
  const frame = encodeTermDataFrame("a1b2c3d4-e5f6-7890-abcd-ef1234567890", payload);
  const parsed = parseTermDataFrame(frame);
  assert.ok(parsed);
  assert.equal(parsed.sessionId, "a1b2c3d4-e5f6-7890-abcd-ef1234567890");
  assert.equal(new TextDecoder().decode(parsed.bytes), "ArubaOS-CX\r\n");
  assert.equal(parseTermDataFrame(new Uint8Array([1, 2, 3])), null);
});

test("isConnectError surfaces Unable to Connect timeout/refused without secrets", () => {
  const timeout = new RpcError("Unable to Connect to 192.0.2.1: connection timed out", -32030, {
    code: "unable_to_connect",
    kind: "unable_to_connect",
    host: "192.0.2.1",
    reason: "connection timed out",
    cause: "timeout",
  });
  const got = isConnectError(timeout);
  assert.ok(got);
  assert.equal(got.cause, "timeout");
  assert.equal(got.host, "192.0.2.1");
  assert.equal(got.reason, "connection timed out");
  assert.equal(got.reason.includes("password"), false);

  const refused = isConnectError(new RpcError("Connection refused"));
  assert.ok(refused);
  assert.equal(refused.cause, "refused");

  const cancelled = isConnectError(new RpcError("cancelled"));
  assert.equal(cancelled, null);
  assert.equal(isCancelled(new RpcError("cancelled")), true);
  assert.equal(isCancelled(new Error("other")), false);

  const hostKey = isConnectError(
    new RpcError("host key untrusted for 10.1.0.12", -32021, {
      code: "host_key_untrusted",
      kind: "host_key_untrusted",
      host: "10.1.0.12",
      presented: "SHA256:abc",
    }),
  );
  assert.equal(hostKey, null);
});

test("isConnectError treats session.open RPC timeout as Unable to Connect", () => {
  const got = isConnectError(new RpcError("RPC timeout: session.open"));
  assert.ok(got);
  assert.equal(got.cause, "timeout");
});

test("appendSessionPrebuffer keeps early banner bytes before pane subscribe", () => {
  const enc = new TextEncoder();
  const a = enc.encode("login: ");
  const b = enc.encode("password: ");
  const merged = appendSessionPrebuffer(a, b);
  assert.equal(new TextDecoder().decode(merged), "login: password: ");
});

test("appendSessionPrebuffer caps to trailing tail", () => {
  const first = new Uint8Array(MAX_SESSION_PREBUFFER_BYTES);
  first.fill(0x41);
  const chunk = new Uint8Array([0x5a, 0x5a, 0x5a]);
  const out = appendSessionPrebuffer(first, chunk);
  assert.equal(out.length, MAX_SESSION_PREBUFFER_BYTES);
  assert.equal(out[out.length - 1], 0x5a);
  assert.equal(out[out.length - 2], 0x5a);
  assert.equal(out[out.length - 3], 0x5a);
});

test("sessionOpenedInfo reads a launcher session and ignores terminal data", () => {
  const info = sessionOpenedInfo({
    event: "session.opened",
    sessionId: "sess-1",
    data: JSON.stringify({
      id: "sess-1",
      device_id: "dev-1",
      name: "pe-1",
      kind: "ssh",
      vendor: "linux",
      connected: true,
      created_at: "2026-01-01T00:00:00Z",
    }),
  });
  assert.equal(info?.id, "sess-1");
  assert.equal(info?.device_id, "dev-1");
  assert.equal(info?.name, "pe-1");
  assert.equal(sessionOpenedInfo({ event: "session.data", data: "bm90IGpzb24=" }), null);
  assert.equal(sessionOpenedInfo({ event: "session.opened", data: "not-json" }), null);
});

test("hostKeyEventInfo reads a containerlab mismatch and ignores terminal data", () => {
  const info = hostKeyEventInfo({
    event: "host_key",
    data: JSON.stringify({
      code: "host_key_mismatch",
      kind: "host_key_mismatch",
      host: "172.20.20.2",
      pinned: "SHA256:old",
      presented: "SHA256:new",
      device_id: "dev-1",
    }),
  });
  assert.equal(info?.mismatch, true);
  assert.equal(info?.host, "172.20.20.2");
  assert.equal(info?.presented, "SHA256:new");
  assert.equal(info?.pinned, "SHA256:old");
  assert.equal(info?.deviceId, "dev-1");
  assert.equal(hostKeyEventInfo({ event: "session.data", data: "bm90IGpzb24=" }), null);
  assert.equal(hostKeyEventInfo({ event: "host_key", data: "not-json" }), null);
});
