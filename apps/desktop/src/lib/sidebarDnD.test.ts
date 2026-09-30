import assert from "node:assert/strict";
import { test } from "node:test";
import {
  deviceDropFolder,
  folderDropDestination,
  readSidebarDrag,
  sameDeviceFolder,
  sidebarDropAllowed,
  writeSidebarDrag,
} from "./sidebarDnD.ts";

function fakeDt(seed: Record<string, string> = {}): DataTransfer {
  const store = new Map(Object.entries(seed));
  return {
    effectAllowed: "none",
    getData: (type: string) => store.get(type) ?? "",
    setData: (type: string, value: string) => {
      store.set(type, value);
    },
  } as unknown as DataTransfer;
}

test("device drag payload survives custom MIME and text/plain fallback", () => {
  const dt = fakeDt();
  writeSidebarDrag(dt, { kind: "device", id: "dev-1" });
  assert.deepEqual(readSidebarDrag(dt), { kind: "device", id: "dev-1" });
  const plainOnly = fakeDt({ "text/plain": "late-device:dev-1" });
  assert.deepEqual(readSidebarDrag(plainOnly), { kind: "device", id: "dev-1" });
});

test("folder drag payload round-trips", () => {
  const dt = fakeDt();
  writeSidebarDrag(dt, { kind: "folder", path: "Sites/NYC" });
  assert.deepEqual(readSidebarDrag(dt), { kind: "folder", path: "Sites/NYC" });
});

test("drop on a folder or sibling device puts the session in that folder", () => {
  assert.equal(deviceDropFolder({ type: "folder", path: "Sites/NYC" }), "Sites/NYC");
  assert.equal(deviceDropFolder({ type: "device", folder: "Sites/NYC" }), "Sites/NYC");
  assert.equal(deviceDropFolder({ type: "device", folder: null }), null);
  assert.equal(deviceDropFolder({ type: "root" }), null);
  assert.equal(deviceDropFolder({ type: "folder", path: "" }), null);
});

test("dragging a folder into itself or a child is rejected", () => {
  assert.equal(folderDropDestination("Sites/NYC", "Sites/NYC/Core"), null);
  assert.equal(folderDropDestination("Sites", "Sites"), "Sites");
  assert.equal(folderDropDestination("Sites/NYC", "Sites"), "Sites/NYC");
  assert.equal(folderDropDestination("Sites/NYC", "Labs"), "Labs/NYC");
  assert.equal(folderDropDestination("Sites/NYC", null), "NYC");
});

test("sameDeviceFolder treats empty and null as Sessions", () => {
  assert.equal(sameDeviceFolder(null, null), true);
  assert.equal(sameDeviceFolder("", null), true);
  assert.equal(sameDeviceFolder("NYC", "NYC"), true);
  assert.equal(sameDeviceFolder("NYC", null), false);
});

test("drop-target rules: Sessions ungroups, device files into that folder, tools reject, nested self illegal", () => {
  const device = { kind: "device" as const, id: "dev-1" };
  const nested = { kind: "folder" as const, path: "Sites/NYC" };
  const sites = { kind: "folder" as const, path: "Sites" };

  assert.equal(sidebarDropAllowed(device, { type: "root" }), true);
  assert.equal(sidebarDropAllowed(device, { type: "folder", path: "" }), true);
  assert.equal(sidebarDropAllowed(device, { type: "device", folder: "Sites/NYC" }), true);
  assert.equal(sidebarDropAllowed(device, { type: "tools" }), false);
  assert.equal(sidebarDropAllowed(nested, { type: "tools" }), false);

  assert.equal(sidebarDropAllowed(nested, { type: "root" }), true);
  assert.equal(sidebarDropAllowed(nested, { type: "folder", path: "Labs" }), true);
  assert.equal(sidebarDropAllowed(nested, { type: "device", folder: "Labs" }), true);

  assert.equal(sidebarDropAllowed(sites, { type: "folder", path: "Sites" }), false);
  assert.equal(sidebarDropAllowed(sites, { type: "folder", path: "Sites/NYC" }), false);
  assert.equal(sidebarDropAllowed(sites, { type: "device", folder: "Sites" }), false);
  assert.equal(sidebarDropAllowed(sites, { type: "device", folder: "Sites/NYC" }), false);
  assert.equal(sidebarDropAllowed(nested, { type: "folder", path: "Sites" }), false);
});
