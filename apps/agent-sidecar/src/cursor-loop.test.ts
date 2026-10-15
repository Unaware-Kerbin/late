import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { trimToolNotes } from "./cursor-loop.ts";

describe("trimToolNotes", () => {
  it("keeps newest notes under the char budget", () => {
    const notes = ["one".repeat(100), "two".repeat(100), "three".repeat(100)];
    const trimmed = trimToolNotes(notes, 400);
    assert.equal(trimmed.length, 1);
    assert.ok(trimmed[0]!.length <= 400);
    assert.ok(notes[notes.length - 1]!.endsWith(trimmed[0]!));
  });

  it("returns empty for empty input", () => {
    assert.deepEqual(trimToolNotes([]), []);
  });
});
