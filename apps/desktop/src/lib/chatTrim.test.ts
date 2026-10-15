import assert from "node:assert/strict";
import test from "node:test";
import {
  MAX_CHAT_CONTENT_CHARS,
  MAX_CHAT_MESSAGES,
  MAX_LOGO_DATA_URL_CHARS,
  capChatContent,
  capChatMessage,
  dedupeChatLogos,
  trimChatMessages,
} from "./chatTrim.ts";
import type { ChatMsg } from "../types";

function msg(partial: Partial<ChatMsg> & { id: string; content: string }): ChatMsg {
  return { role: "assistant", ...partial };
}

test("capChatContent leaves short text alone", () => {
  assert.equal(capChatContent("hello"), "hello");
});

test("capChatContent truncates long text with marker", () => {
  const big = "x".repeat(MAX_CHAT_CONTENT_CHARS + 500);
  const out = capChatContent(big);
  assert.ok(out.length < big.length);
  assert.match(out, /truncated 500 chars/);
  assert.ok(out.startsWith("x".repeat(100)));
});

test("capChatMessage drops oversized logoDataUrl", () => {
  const logo = "data:image/png;base64," + "A".repeat(MAX_LOGO_DATA_URL_CHARS + 10);
  const out = capChatMessage(msg({ id: "1", content: "hi", logoDataUrl: logo }));
  assert.equal(out.content, "hi");
  assert.equal(out.logoDataUrl, undefined);
});

test("dedupeChatLogos keeps only the newest copy of each logo", () => {
  const logo = "data:image/png;base64,abc";
  const rows = dedupeChatLogos([
    msg({ id: "a", content: "one", logoDataUrl: logo }),
    msg({ id: "b", content: "two", logoDataUrl: logo }),
    msg({ id: "c", content: "three", logoDataUrl: "data:image/png;base64,other" }),
  ]);
  assert.equal(rows[0].logoDataUrl, undefined);
  assert.equal(rows[1].logoDataUrl, logo);
  assert.equal(rows[2].logoDataUrl, "data:image/png;base64,other");
});

test("trimChatMessages caps count and content", () => {
  const rows: ChatMsg[] = [];
  for (let i = 0; i < MAX_CHAT_MESSAGES + 5; i++) {
    rows.push(msg({ id: String(i), content: "x".repeat(MAX_CHAT_CONTENT_CHARS + 20), logoDataUrl: "data:logo" }));
  }
  const out = trimChatMessages(rows);
  assert.equal(out.length, MAX_CHAT_MESSAGES);
  assert.equal(out[0].id, "5");
  assert.ok(out.every((m) => m.content.length <= MAX_CHAT_CONTENT_CHARS + 40));
  const withLogo = out.filter((m) => m.logoDataUrl);
  assert.equal(withLogo.length, 1);
  assert.equal(withLogo[0].id, out[out.length - 1].id);
});
