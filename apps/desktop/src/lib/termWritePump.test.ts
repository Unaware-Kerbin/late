import assert from "node:assert/strict";
import test from "node:test";
import { MAX_WRITE_PENDING_CHARS, appendWritePending } from "./termWritePump.ts";

test("appendWritePending concatenates under the cap", () => {
  assert.equal(appendWritePending("ab", "cd", 10), "abcd");
});

test("appendWritePending keeps only the tail when over cap", () => {
  const pending = "a".repeat(100);
  const next = appendWritePending(pending, "b".repeat(50), 80);
  assert.equal(next.length, 80);
  assert.ok(next.endsWith("b".repeat(50)));
  assert.ok(next.startsWith("a".repeat(30)));
});

test("default pending cap is 512k", () => {
  assert.equal(MAX_WRITE_PENDING_CHARS, 512_000);
  const huge = "z".repeat(MAX_WRITE_PENDING_CHARS + 1000);
  const out = appendWritePending("", huge);
  assert.equal(out.length, MAX_WRITE_PENDING_CHARS);
});
