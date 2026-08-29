import assert from "node:assert/strict";
import test from "node:test";
import {
  isSshInventoryDevice,
  readStageArt,
  resolvePathDeviceId,
  sessionDeviceId,
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

test("serial session is not a PATH inventory target", () => {
  const serialSess: SessionInfo = { ...sshSess, id: "ser-1", kind: "serial", device_id: "dev-console" };
  assert.equal(resolvePathDeviceId("", "ser-1", [serialSess], [sshDev, serialDev]), "");
});
