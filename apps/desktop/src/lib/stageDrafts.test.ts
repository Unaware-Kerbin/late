import assert from "node:assert/strict";
import test from "node:test";
import {
  isSshInventoryDevice,
  readStageArt,
  resolveLivePushSession,
  resolvePathDeviceId,
  sessionDeviceId,
  stageBodyAfterOpen,
  stageDeviceAfterOpen,
  stageOpenFromToolDetail,
  stageSessionAfterOpen,
  stagingSeedRemounts,
} from "./stageDrafts.ts";
import type { Device, SessionInfo } from "../types.ts";

const sshDev: Device = {
  id: "dev-aruba",
  name: "lab-aruba",
  kind: "ssh",
  vendor: "aos_cx",
  host: "192.0.2.80",
  port: 22,
  tags: [],
  syntax_highlight: true,
  quick_copy: true,
  legacy_ssh: false,
};

const serialDev: Device = {
  ...sshDev,
  id: "dev-console",
  name: "console",
  kind: "serial",
  host: null,
  serial_path: "/dev/ttyUSB0",
};

const sshSess: SessionInfo = {
  id: "ssh-sess-1",
  device_id: "dev-aruba",
  name: "aruba ssh",
  kind: "ssh",
  vendor: "aos_cx",
  connected: true,
  created_at: "",
};

test("readStageArt restores camelCase and snake_case ids", () => {
  const a = readStageArt({
    id: "draft-1",
    format: "ansible",
    intent: "configure VLAN 2000",
    body: "vlan 2000\n",
    vendor: "aos_cx",
    ask_operator: "check vendor",
    device_id: "dev-aruba",
    sessionId: "ssh-sess-1",
  });
  assert.equal(a.id, "draft-1");
  assert.equal(a.format, "ansible");
  assert.equal(a.deviceId, "dev-aruba");
  assert.equal(a.sessionId, "ssh-sess-1");
  assert.equal(a.ask, "check vendor");
});

test("PATH device comes from SSH session when the saved device id is missing", () => {
  assert.equal(sessionDeviceId(sshSess), "dev-aruba");
  assert.equal(isSshInventoryDevice(sshDev), true);
  assert.equal(isSshInventoryDevice(serialDev), false);
  assert.equal(resolvePathDeviceId("", "ssh-sess-1", [sshSess], [sshDev, serialDev]), "dev-aruba");
  assert.equal(resolvePathDeviceId("", "ssh-sess-1", [sshSess], [serialDev]), "");
});

test("propose_staged_artifact detail with body opens staging content including vlan 2500", () => {
  const seed = stageOpenFromToolDetail({
    id: "a5b867fa-1111-2222-3333-444455556666",
    format: "cli",
    intent: "configure VLAN 2500",
    body: "vlan 2500\nname VLAN2500\n",
    vendor: "aos_cx",
    session_id: "a001b507-0d23-4574-8fc9-0cbd8fbeca72",
    device_id: "dev-aruba",
  });
  assert.equal(seed.format, "cli");
  assert.equal(seed.intent, "configure VLAN 2500");
  assert.match(seed.body ?? "", /vlan 2500/);
  assert.match(seed.body ?? "", /name VLAN2500/);
  assert.equal(seed.stageId, "a5b867fa-1111-2222-3333-444455556666");
  assert.equal(seed.sessionId, "a001b507-0d23-4574-8fc9-0cbd8fbeca72");
  assert.equal(seed.deviceId, "dev-aruba");
});

test("MCP body seed remounts Staging; Push save by id does not", () => {
  const seed = stageOpenFromToolDetail({
    id: "draft-1",
    format: "cli",
    intent: "configure VLAN 2500",
    body: "vlan 2500\nname VLAN2500\n",
    session_id: "a001b507-0d23-4574-8fc9-0cbd8fbeca72",
  });
  assert.equal(stagingSeedRemounts(seed), true);
  assert.equal(stagingSeedRemounts({ stageId: seed.stageId, format: seed.format }), false);
  assert.equal(stagingSeedRemounts({ clear: true }), true);
  assert.equal(stagingSeedRemounts({}), false);
  assert.equal(
    stageBodyAfterOpen({ stageId: "draft-1", stageBody: seed.body }, { stageId: "draft-2" }),
    undefined,
  );
  assert.match(
    stageBodyAfterOpen({ stageId: "draft-1", stageBody: "old\n" }, seed) ?? "",
    /vlan 2500/,
  );
  assert.equal(
    stageBodyAfterOpen({ stageId: "draft-1", stageBody: seed.body }, { stageId: "draft-1" }),
    seed.body,
  );
  assert.equal(
    stageSessionAfterOpen({ stageId: "draft-1", stageSessionId: seed.sessionId }, { stageId: "draft-2" }),
    undefined,
  );
  assert.equal(
    stageSessionAfterOpen({ stageId: "draft-1", stageSessionId: seed.sessionId }, { stageId: "draft-1" }),
    seed.sessionId,
  );
  assert.equal(
    stageSessionAfterOpen({ stageId: "draft-1", stageSessionId: "old-sess" }, seed),
    seed.sessionId,
  );
  assert.equal(stageDeviceAfterOpen({ stageId: "draft-1", deviceId: "dev-old" }, { stageId: "draft-2" }), undefined);
  assert.equal(stageDeviceAfterOpen({ stageId: "draft-1", deviceId: "dev-old" }, { stageId: "draft-1" }), "dev-old");
  assert.equal(stageDeviceAfterOpen({ stageId: "draft-1", deviceId: "dev-old" }, { clear: true }), "dev-old");
  assert.equal(
    stageDeviceAfterOpen({ stageId: "draft-1", deviceId: "dev-old" }, { ...seed, deviceId: undefined }),
    "dev-old",
  );
  assert.equal(
    stageDeviceAfterOpen({ stageId: "other", deviceId: "dev-old" }, { ...seed, deviceId: "dev-aruba" }),
    "dev-aruba",
  );
});

test("serial session is not a PATH inventory target", () => {
  const serialSess: SessionInfo = { ...sshSess, id: "ser-1", kind: "serial", device_id: "dev-console" };
  assert.equal(resolvePathDeviceId("", "ser-1", [serialSess], [sshDev, serialDev]), "");
});

test("Push CLI dropdown uses a live PTY, not a stale seed UUID", () => {
  const live = { ...sshSess, id: "ece3d42c-4363-435b-8678-9a98685f161c" };
  assert.equal(resolveLivePushSession(live.id, [live]), live.id);
  assert.equal(resolveLivePushSession("a001b507-0d23-4574-8fc9-0cbd8fbeca72", [live]), live.id);
  assert.equal(resolveLivePushSession("", [live]), live.id);
  assert.equal(resolveLivePushSession("", []), "");
  const other: SessionInfo = { ...live, id: "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb", name: "core" };
  assert.equal(resolveLivePushSession("", [live, other]), "");
  assert.equal(resolveLivePushSession(other.id, [live, other]), other.id);
  assert.equal(resolveLivePushSession("dead-uuid", [live, other]), "");
});
