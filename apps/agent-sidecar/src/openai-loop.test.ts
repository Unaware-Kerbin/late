import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { vllmRejectsAutoToolChoice } from "./openai-loop.ts";

describe("vLLM tool_choice auto", () => {
  it("retries when the box was started without a tool-call parser", () => {
    const body =
      '{"message":"\\"auto\\" tool choice requires --enable-auto-tool-choice and --tool-call-parser to be set"}';
    assert.equal(vllmRejectsAutoToolChoice(400, body), true);
    assert.equal(vllmRejectsAutoToolChoice(401, body), false);
    assert.equal(vllmRejectsAutoToolChoice(400, '{"message":"context length"}'), false);
  });
});
