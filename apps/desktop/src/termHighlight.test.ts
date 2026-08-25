import assert from "node:assert/strict";
import test from "node:test";
import {
  colorAnsiAware,
  colorPlain,
  DEFAULT_TERM_HIGHLIGHTS,
  holdPlainSuffix,
  resolvedRules,
  sanitizeHighlights,
  StreamHighlighter,
  type TermHighlightRule,
} from "./termHighlight.ts";

const down: TermHighlightRule = {
  id: "down",
  pattern: "down",
  scheme: "custom",
  fg: "#ff5c7a",
  wholeWord: true,
  caseSensitive: false,
};
const up: TermHighlightRule = {
  id: "up",
  pattern: "up",
  scheme: "custom",
  fg: "#3ddc8a",
  wholeWord: true,
  caseSensitive: false,
};
const admin: TermHighlightRule = {
  id: "admin",
  pattern: "administratively down",
  scheme: "custom",
  fg: "#ff5c7a",
  wholeWord: false,
  caseSensitive: false,
};

test("colors whole-word down and up", () => {
  const out = colorPlain("line protocol is down, status is up", [down, up]);
  assert.match(out, /\x1b\[38;2;255;92;122mdown\x1b\[39m/);
  assert.match(out, /\x1b\[38;2;61;220;138mup\x1b\[39m/);
});

test("whole word skips shutdown and uptime", () => {
  const out = colorPlain("shutdown uptime", [down, up]);
  assert.equal(out, "shutdown uptime");
});

test("longer keyword wins over down", () => {
  const out = colorPlain("is administratively down", [down, admin]);
  assert.match(out, /administratively down/);
  assert.doesNotMatch(out, /\[38;2;255;92;122mdown/);
});

test("does not rewrite CSI sequences", () => {
  const raw = "\x1b[32mhello\x1b[0m down";
  const out = colorAnsiAware(raw, [down]);
  assert.equal(out.startsWith("\x1b[32mhello\x1b[0m "), true);
  assert.match(out, /\x1b\[38;2;255;92;122mdown\x1b\[39m/);
});

test("hold-back joins down across chunks", () => {
  const hl = new StreamHighlighter({ enabled: true, schemes: DEFAULT_TERM_HIGHLIGHTS.schemes, rules: [down] });
  const a = hl.feed("protocol is do");
  assert.equal(a.includes("down"), false);
  const b = hl.feed("wn\r\n");
  const all = a + b + hl.flush();
  assert.match(all, /\x1b\[38;2;255;92;122mdown\x1b\[39m/);
});

test("disabled highlighter is a passthrough", () => {
  const hl = new StreamHighlighter({ enabled: false, schemes: DEFAULT_TERM_HIGHLIGHTS.schemes, rules: [down] });
  assert.equal(hl.feed("is down"), "is down");
});

test("sanitize drops junk and caps length", () => {
  const s = sanitizeHighlights({
    enabled: true,
    rules: [{ pattern: "down", fg: "red" }, { pattern: "x".repeat(200), fg: "#00ff00" }],
  });
  assert.equal(s.rules[0].fg, "#ff5c7a");
  assert.equal(s.rules[1].pattern.length, 64);
  assert.equal(s.schemes.down.fg, "#ff5c7a");
});

test("changing the down scheme recolors down keywords", () => {
  const s = sanitizeHighlights({
    enabled: true,
    schemes: { down: { fg: "#5b9dff" }, up: { fg: "#ff9f43" } },
    rules: DEFAULT_TERM_HIGHLIGHTS.rules,
  });
  const resolved = resolvedRules(s);
  const downRule = resolved.find((r) => r.pattern === "down");
  const upRule = resolved.find((r) => r.pattern === "up");
  assert.equal(downRule?.fg, "#5b9dff");
  assert.equal(upRule?.fg, "#ff9f43");
  const out = colorPlain("is down / is up", resolved);
  assert.match(out, /\x1b\[38;2;91;157;255mdown/);
  assert.match(out, /\x1b\[38;2;255;159;67mup/);
});

test("defaults include down and up", () => {
  const pats = DEFAULT_TERM_HIGHLIGHTS.rules.map((r) => r.pattern);
  assert.ok(pats.includes("down"));
  assert.ok(pats.includes("up"));
});

test("holdPlainSuffix keeps a keyword prefix", () => {
  const { flush, hold } = holdPlainSuffix("is do", [down]);
  assert.equal(flush, "is ");
  assert.equal(hold, "do");
});
