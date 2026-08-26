import assert from "node:assert/strict";
import test from "node:test";
import {
  STAGE_SECRET_TOKENS,
  stageSecretHits,
  stageSuggestions,
  STAGE_FORMATS,
} from "./stageEditorLang.ts";

test("secret hits flag password-like lines", () => {
  const hits = stageSecretHits("ok\nansible_ssh_pass: nope\n");
  assert.equal(hits.length, 1);
  assert.equal(hits[0].line, 2);
});

test("suggestions never include secret tokens", () => {
  for (const format of STAGE_FORMATS) {
    const blob = stageSuggestions(format)
      .map((s) => `${s.label}\n${s.insertText}`)
      .join("\n")
      .toLowerCase();
    for (const tok of STAGE_SECRET_TOKENS) {
      assert.equal(blob.includes(tok), false, `${format} suggestion leaked ${tok}`);
    }
  }
});
