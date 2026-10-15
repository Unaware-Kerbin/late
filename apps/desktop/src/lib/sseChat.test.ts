import assert from "node:assert/strict";
import test from "node:test";
import { MAX_SSE_BUF_CHARS, capIncompleteSseBuffer, ingestSseChatChunk } from "./sseChat.ts";

test("ingestSseChatChunk parses complete Local AI delta/tool/done frames", () => {
  const chunk =
    `data: ${JSON.stringify({ type: "delta", text: "hi" })}\n\n` +
    `data: ${JSON.stringify({ type: "tool", name: "propose_command", status: "start" })}\n\n` +
    `data: ${JSON.stringify({ type: "done", message: "ok" })}\n\n`;
  const { buf, events } = ingestSseChatChunk("", chunk);
  assert.equal(buf, "");
  assert.equal(events.length, 3);
  assert.equal(events[0]?.type, "delta");
  assert.equal(events[1]?.type, "tool");
  assert.equal(events[2]?.type, "done");
});

test("ingestSseChatChunk keeps an incomplete frame across chunks", () => {
  const first = ingestSseChatChunk("", `data: {"type":"delta","text":"hel`);
  assert.equal(first.events.length, 0);
  assert.match(first.buf, /hel$/);
  const second = ingestSseChatChunk(first.buf, `lo"}\n\n`);
  assert.equal(second.buf, "");
  assert.equal(second.events.length, 1);
  assert.deepEqual(second.events[0], { type: "delta", text: "hello" });
});

test("capIncompleteSseBuffer drops whole frames from the front, not mid-JSON", () => {
  const small = `data: ${JSON.stringify({ type: "heartbeat", message: "x" })}\n\n`;
  const incomplete = `data: {"type":"delta","text":"${"y".repeat(100)}`;
  const buf = small.repeat(3) + incomplete;
  // Force overflow so at least one completed frame must be dropped.
  const capped = capIncompleteSseBuffer(buf, incomplete.length + Math.floor(small.length / 2));
  assert.ok(capped.startsWith("data:"), capped.slice(0, 40));
  assert.ok(capped.length < buf.length, "front frames were dropped");
  assert.ok(capped.includes('"type":"delta"'), "must not slice inside the JSON key");
  assert.equal(capped.endsWith(incomplete.slice(-20)), true);
});

test("oversized single incomplete event is discarded (no corrupt JSON tail)", () => {
  const huge = `data: {"type":"delta","text":"${"z".repeat(MAX_SSE_BUF_CHARS + 50)}`;
  assert.equal(capIncompleteSseBuffer(huge), "");
  // Completing after discard must not revive a truncated prefix into a fake event.
  const { events } = ingestSseChatChunk("", huge + `"}\n\n`);
  assert.equal(events.length, 0);
});

test("complete Approve frames survive the same chunk as a huge incomplete delta", () => {
  const approval =
    `data: ${JSON.stringify({
      type: "approval",
      pending: { proposalId: "p1", kind: "command", title: "Approve" },
    })}\n\n`;
  const incomplete = `data: {"type":"delta","text":"${"y".repeat(5_000)}`;
  const { buf, events } = ingestSseChatChunk("", approval + incomplete, 8_000);
  assert.equal(events.length, 1);
  assert.equal(events[0]?.type, "approval");
  assert.ok(buf.startsWith("data:"), "incomplete delta kept for next chunk");
  assert.ok(!buf.includes("approval"), "approval already emitted, not left in buf");
});

test("MAX_SSE_BUF_CHARS stays at 1MiB chars", () => {
  assert.equal(MAX_SSE_BUF_CHARS, 1_000_000);
});
