import assert from "node:assert/strict";
import test from "node:test";
import { vendorLabel } from "../types.ts";
import {
  addPermitToken,
  coercePermitPolicy,
  coercePermitPolicyList,
  isDangerousPermitVerb,
  normalizePermitToken,
  removePermitToken,
} from "./permitList.ts";

test("MCP grant-folder is not this permit list", () => {
  assert.equal(vendorLabel("aos_cx"), "AOS-CX (Aruba)");
  assert.equal(vendorLabel("arista_eos"), "EOS (Arista)");
  assert.equal(vendorLabel("cisco_ios"), "Cisco IOS");
  const view = coercePermitPolicy({
    vendor: "aos_cx",
    unrestricted: false,
    allowAlwaysAllow: true,
    allow: ["show", "vlan", "name"],
    builtinAllow: [],
    deny: ["erase", "reload", "start-shell"],
    denySubstrings: ["shell"],
    path: "/home/me/.config/late/policies/aos_cx.yaml",
  });
  assert.ok(view);
  assert.equal(view.path.includes("policies/aos_cx.yaml"), true);
  assert.equal(view.path.includes("grant"), false);
  assert.match(view.path, /aos_cx\.yaml$/);
});

test("save/load coerce snake_case and camelCase", () => {
  const snake = coercePermitPolicy({
    vendor: "aos_cx",
    unrestricted: false,
    allow_always_allow: true,
    allow: ["show", "name"],
    builtin_allow: ["exit"],
    deny: ["reload"],
    deny_substrings: ["erase"],
    path: "/tmp/late/policies/aos_cx.yaml",
  });
  assert.ok(snake);
  assert.deepEqual(snake.allow, ["show", "name"]);
  assert.deepEqual(snake.builtinAllow, ["exit"]);
  assert.deepEqual(snake.deny, ["reload"]);
  const list = coercePermitPolicyList({
    policies: [{ vendor: "linux", unrestricted: true, allow_always_allow: true, allow: ["ls"] }],
  });
  assert.equal(list.length, 1);
  assert.equal(list[0].vendor, "linux");
  assert.equal(list[0].unrestricted, true);
  assert.equal(list[0].allowAlwaysAllow, false);
});

test("add and remove allow tokens", () => {
  const a = addPermitToken(["show", "vlan"], "name");
  assert.ok(!("error" in a));
  assert.deepEqual(a.allow, ["show", "vlan", "name"]);
  assert.deepEqual(removePermitToken(a.allow, "vlan"), ["show", "name"]);
  const dup = addPermitToken(["show"], "SHOW");
  assert.ok(!("error" in dup));
  assert.deepEqual(dup.allow, ["show"]);
  assert.equal(normalizePermitToken(" name VLAN "), "name VLAN");
  assert.equal(normalizePermitToken("show | include"), null);
  assert.equal(normalizePermitToken("reload\nerase"), null);
});

test("dangerous verbs warn; name is not dangerous", () => {
  assert.equal(isDangerousPermitVerb("reload"), true);
  assert.equal(isDangerousPermitVerb("erase"), true);
  assert.equal(isDangerousPermitVerb("start-shell"), true);
  assert.equal(isDangerousPermitVerb("name"), false);
  assert.equal(isDangerousPermitVerb("vlan"), false);
});
