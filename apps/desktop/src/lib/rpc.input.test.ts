import assert from "node:assert/strict";
import { test } from "node:test";
import { encodeTermDataFrame, encodeTermInputFrame, parseTermDataFrame, isCancelled, isConnectError, RpcError } from "./rpc";

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
