import assert from "node:assert/strict";
import test from "node:test";
import { polishChatText } from "./chatMarkdown.tsx";

test("polishChatText strips leading assistant role dump", () => {
  const out = polishChatText("assistant\nScrollback shows interfaces.");
  assert.equal(out.startsWith("assistant"), false);
  assert.match(out, /^Scrollback shows interfaces/);
});

test("polishChatText inserts space before glued hostname digits", () => {
  const out = polishChatText("I'll pull the brief from6200F on vlan10.");
  assert.match(out, /from 6200F/);
  assert.match(out, /vlan 10/);
});

test("polishChatText leaves fenced code alone", () => {
  const out = polishChatText("See:\n```\nfrom6200F\nvlan10\n```\nDone.");
  assert.match(out, /```\nfrom6200F\nvlan10\n```/);
  assert.match(out, /Done\./);
});

test("polishChatText does not split IPv4 suffix oddly", () => {
  const out = polishChatText("Use IPv4 on the SVI.");
  assert.match(out, /IPv4/);
});
