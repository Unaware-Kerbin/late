/** Display-only keyword colors for SSH/serial/local xterm. Never sent to the daemon or agent. */

export type HighlightSchemeId = "down" | "up" | "warn" | "custom";

export type HighlightScheme = {
  fg: string;
  bg?: string;
};

export type TermHighlightRule = {
  id: string;
  pattern: string;
  scheme: HighlightSchemeId;
  fg: string;
  bg?: string;
  bold?: boolean;
  wholeWord: boolean;
  caseSensitive: boolean;
};

export type TermHighlightSettings = {
  enabled: boolean;
  schemes: {
    down: HighlightScheme;
    up: HighlightScheme;
    warn: HighlightScheme;
  };
  rules: TermHighlightRule[];
};

export const DEFAULT_SCHEMES: TermHighlightSettings["schemes"] = {
  down: { fg: "#ff5c7a" },
  up: { fg: "#3ddc8a" },
  warn: { fg: "#f0c14a" },
};

const STORE_KEY = "late.termHighlights";
const MAX_PATTERN = 64;

export const HIGHLIGHT_PRESETS: { name: string; fg: string }[] = [
  { name: "Red", fg: "#ff5c7a" },
  { name: "Green", fg: "#3ddc8a" },
  { name: "Yellow", fg: "#f0c14a" },
  { name: "Orange", fg: "#ff9f43" },
  { name: "Cyan", fg: "#4fd1c5" },
  { name: "Magenta", fg: "#c084fc" },
  { name: "Blue", fg: "#5b9dff" },
  { name: "White", fg: "#f5f5f5" },
];

export const SCHEME_LABELS: Record<Exclude<HighlightSchemeId, "custom">, string> = {
  down: "Down / bad",
  up: "Up / good",
  warn: "Warn",
};

function rule(pattern: string, scheme: Exclude<HighlightSchemeId, "custom">, wholeWord: boolean): TermHighlightRule {
  return {
    id: `hl-${pattern.replace(/[^a-z0-9]+/gi, "-")}`,
    pattern,
    scheme,
    fg: DEFAULT_SCHEMES[scheme].fg,
    wholeWord,
    caseSensitive: false,
  };
}

export const DEFAULT_TERM_HIGHLIGHTS: TermHighlightSettings = {
  enabled: true,
  schemes: {
    down: { ...DEFAULT_SCHEMES.down },
    up: { ...DEFAULT_SCHEMES.up },
    warn: { ...DEFAULT_SCHEMES.warn },
  },
  rules: [
    rule("administratively down", "down", false),
    rule("err-disabled", "down", false),
    rule("errdisabled", "down", true),
    rule("down", "down", true),
    rule("failures", "down", true),
    rule("failed", "down", true),
    rule("never", "down", true),
    rule("error", "down", true),
    rule("denied", "down", true),
    rule("unreachable", "down", true),
    rule("up", "up", true),
    rule("connected", "up", true),
    rule("established", "up", true),
    rule("timeout", "warn", true),
    rule("warning", "warn", true),
  ],
};

export function newHighlightRule(): TermHighlightRule {
  return {
    id: `hl-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`,
    pattern: "",
    scheme: "custom",
    fg: DEFAULT_SCHEMES.down.fg,
    wholeWord: true,
    caseSensitive: false,
  };
}

function isHexColor(s: string): boolean {
  return /^#[0-9a-fA-F]{6}$/.test(s);
}

function parseScheme(raw: unknown, fallback: HighlightScheme): HighlightScheme {
  if (!raw || typeof raw !== "object") return { fg: fallback.fg };
  const o = raw as Record<string, unknown>;
  const fg = isHexColor(String(o.fg ?? "")) ? String(o.fg) : fallback.fg;
  const bgRaw = String(o.bg ?? "");
  const bg = isHexColor(bgRaw) ? bgRaw : undefined;
  return bg ? { fg, bg } : { fg };
}

function inferScheme(pattern: string, fg: string): HighlightSchemeId {
  const p = pattern.trim().toLowerCase();
  if (
    /^(administratively down|err-disabled|errdisabled|down|failures|failed|never|error|denied|unreachable)$/.test(p)
  ) {
    return "down";
  }
  if (/^(up|connected|established)$/.test(p)) return "up";
  if (/^(timeout|warning)$/.test(p)) return "warn";
  const f = fg.toLowerCase();
  if (f === DEFAULT_SCHEMES.down.fg) return "down";
  if (f === DEFAULT_SCHEMES.up.fg) return "up";
  if (f === DEFAULT_SCHEMES.warn.fg) return "warn";
  return "custom";
}

function parseSchemeId(raw: unknown, pattern: string, fg: string): HighlightSchemeId {
  const s = String(raw ?? "");
  if (s === "down" || s === "up" || s === "warn" || s === "custom") return s;
  return inferScheme(pattern, fg);
}

export function sanitizeHighlights(raw: unknown): TermHighlightSettings {
  if (!raw || typeof raw !== "object") return structuredClone(DEFAULT_TERM_HIGHLIGHTS);
  const o = raw as Record<string, unknown>;
  const schemesIn = o.schemes && typeof o.schemes === "object" ? (o.schemes as Record<string, unknown>) : {};
  const schemes = {
    down: parseScheme(schemesIn.down, DEFAULT_SCHEMES.down),
    up: parseScheme(schemesIn.up, DEFAULT_SCHEMES.up),
    warn: parseScheme(schemesIn.warn, DEFAULT_SCHEMES.warn),
  };
  const rulesIn = Array.isArray(o.rules) ? o.rules : [];
  const rules: TermHighlightRule[] = [];
  for (const item of rulesIn) {
    if (!item || typeof item !== "object") continue;
    const r = item as Record<string, unknown>;
    const pattern = String(r.pattern ?? "").slice(0, MAX_PATTERN);
    const fgRaw = String(r.fg ?? "");
    const fg = isHexColor(fgRaw) ? fgRaw : DEFAULT_SCHEMES.down.fg;
    const bgRaw = String(r.bg ?? "");
    const bg = isHexColor(bgRaw) ? bgRaw : undefined;
    const scheme = parseSchemeId(r.scheme, pattern, fg);
    rules.push({
      id: String(r.id ?? newHighlightRule().id).slice(0, 80),
      pattern,
      scheme,
      fg,
      bg,
      bold: Boolean(r.bold),
      wholeWord: Boolean(r.wholeWord ?? r.whole_word ?? true),
      caseSensitive: Boolean(r.caseSensitive ?? r.case_sensitive),
    });
    if (rules.length >= 80) break;
  }
  return { enabled: o.enabled !== false, schemes, rules };
}

/** Apply Down/Up/Warn scheme colors onto each rule. Custom keeps its own swatch. */
export function resolvedRules(s: TermHighlightSettings): TermHighlightRule[] {
  const schemes = s.schemes ?? DEFAULT_SCHEMES;
  return (s.rules ?? []).map((r) => {
    if (!r.scheme || r.scheme === "custom") return r;
    const sch = schemes[r.scheme] ?? DEFAULT_SCHEMES.down;
    return { ...r, fg: sch.fg, bg: sch.bg };
  });
}

export function loadTermHighlights(): TermHighlightSettings {
  try {
    const raw = localStorage.getItem(STORE_KEY);
    if (!raw) return structuredClone(DEFAULT_TERM_HIGHLIGHTS);
    return sanitizeHighlights(JSON.parse(raw));
  } catch {
    return structuredClone(DEFAULT_TERM_HIGHLIGHTS);
  }
}

export function persistTermHighlights(s: TermHighlightSettings) {
  try {
    localStorage.setItem(STORE_KEY, JSON.stringify(sanitizeHighlights(s)));
  } catch {
    /* ignore */
  }
}

function rgb(hex: string): [number, number, number] | null {
  if (!isHexColor(hex)) return null;
  return [parseInt(hex.slice(1, 3), 16), parseInt(hex.slice(3, 5), 16), parseInt(hex.slice(5, 7), 16)];
}

function wrap(text: string, rule: TermHighlightRule): string {
  const fg = rgb(rule.fg);
  const bg = rule.bg ? rgb(rule.bg) : null;
  let out = "";
  if (rule.bold) out += "\x1b[1m";
  if (fg) out += `\x1b[38;2;${fg[0]};${fg[1]};${fg[2]}m`;
  if (bg) out += `\x1b[48;2;${bg[0]};${bg[1]};${bg[2]}m`;
  out += text;
  if (rule.bold) out += "\x1b[22m";
  if (fg) out += "\x1b[39m";
  if (bg) out += "\x1b[49m";
  return out;
}

function isWordChar(ch: string | undefined): boolean {
  if (!ch) return false;
  return /[A-Za-z0-9_]/.test(ch);
}

type Compiled = TermHighlightRule & { needle: string };

function compile(rules: TermHighlightRule[]): Compiled[] {
  const out: Compiled[] = [];
  for (const r of rules) {
    const pattern = r.pattern.trim();
    if (!pattern) continue;
    out.push({ ...r, pattern, needle: r.caseSensitive ? pattern : pattern.toLowerCase() });
  }
  out.sort((a, b) => b.pattern.length - a.pattern.length);
  return out;
}

function findMatches(text: string, rules: Compiled[]): { start: number; end: number; rule: Compiled }[] {
  const hits: { start: number; end: number; rule: Compiled }[] = [];
  for (const rule of rules) {
    const hay = rule.caseSensitive ? text : text.toLowerCase();
    const needle = rule.needle;
    if (!needle) continue;
    let from = 0;
    while (from <= hay.length - needle.length) {
      const i = hay.indexOf(needle, from);
      if (i < 0) break;
      const end = i + needle.length;
      if (rule.wholeWord && (isWordChar(text[i - 1]) || isWordChar(text[end]))) {
        from = i + 1;
        continue;
      }
      hits.push({ start: i, end, rule });
      from = i + 1;
    }
  }
  hits.sort((a, b) => a.start - b.start || b.end - a.end - (a.end - a.start));
  const picked: typeof hits = [];
  let cursor = 0;
  for (const h of hits) {
    if (h.start < cursor) continue;
    picked.push(h);
    cursor = h.end;
  }
  return picked;
}

export function colorPlain(text: string, rules: TermHighlightRule[]): string {
  const compiled = compile(rules);
  if (!compiled.length || !text) return text;
  const hits = findMatches(text, compiled);
  if (!hits.length) return text;
  let out = "";
  let i = 0;
  for (const h of hits) {
    out += text.slice(i, h.start);
    out += wrap(text.slice(h.start, h.end), h.rule);
    i = h.end;
  }
  out += text.slice(i);
  return out;
}

/** Returns the end index of a complete escape, or -1 if incomplete. */
export function consumeEscape(s: string, i: number): number {
  if (s[i] !== "\x1b") return i;
  if (i + 1 >= s.length) return -1;
  const n = s[i + 1];
  if (n === "[") {
    let j = i + 2;
    while (j < s.length) {
      const c = s.charCodeAt(j);
      if (c >= 0x30 && c <= 0x3f) {
        j++;
        continue;
      }
      break;
    }
    while (j < s.length) {
      const c = s.charCodeAt(j);
      if (c >= 0x20 && c <= 0x2f) {
        j++;
        continue;
      }
      break;
    }
    if (j >= s.length) return -1;
    const c = s.charCodeAt(j);
    if (c >= 0x40 && c <= 0x7e) return j + 1;
    return j + 1;
  }
  if (n === "]") {
    let j = i + 2;
    while (j < s.length) {
      if (s[j] === "\x07") return j + 1;
      if (s[j] === "\x1b" && s[j + 1] === "\\") return j + 2;
      j++;
    }
    return -1;
  }
  if (i + 2 <= s.length) return i + 2;
  return -1;
}

export function colorAnsiAware(s: string, rules: TermHighlightRule[]): string {
  if (!rules.length || !s) return s;
  let out = "";
  let i = 0;
  let plain = "";
  const flushPlain = () => {
    if (plain) {
      out += colorPlain(plain, rules);
      plain = "";
    }
  };
  while (i < s.length) {
    if (s[i] === "\x1b") {
      const end = consumeEscape(s, i);
      if (end < 0) {
        flushPlain();
        out += s.slice(i);
        return out;
      }
      flushPlain();
      out += s.slice(i, end);
      i = end;
      continue;
    }
    plain += s[i];
    i++;
  }
  flushPlain();
  return out;
}

function lastPlainStart(s: string): number {
  let i = 0;
  let last = 0;
  while (i < s.length) {
    if (s[i] === "\x1b") {
      const end = consumeEscape(s, i);
      if (end < 0) return i;
      last = end;
      i = end;
      continue;
    }
    i++;
  }
  return last;
}

function isPatternPrefix(suffix: string, rules: Compiled[]): boolean {
  if (!suffix) return false;
  for (const r of rules) {
    const pat = r.caseSensitive ? r.pattern : r.pattern.toLowerCase();
    const suf = r.caseSensitive ? suffix : suffix.toLowerCase();
    if (pat.startsWith(suf)) return true;
  }
  return false;
}

/** Keep a trailing prefix of a keyword so "do"+"wn" still colors `down`. */
export function holdPlainSuffix(ready: string, rules: TermHighlightRule[]): { flush: string; hold: string } {
  const compiled = compile(rules);
  if (!compiled.length || !ready) return { flush: ready, hold: "" };
  const maxLen = Math.min(MAX_PATTERN, Math.max(1, ...compiled.map((r) => r.pattern.length)));
  const headAt = lastPlainStart(ready);
  const head = ready.slice(0, headAt);
  const tail = ready.slice(headAt);
  const limit = Math.min(tail.length, maxLen);
  for (let n = limit; n >= 1; n--) {
    const suffix = tail.slice(tail.length - n);
    if (isPatternPrefix(suffix, compiled)) {
      return { flush: head + tail.slice(0, tail.length - n), hold: suffix };
    }
  }
  return { flush: ready, hold: "" };
}

export class StreamHighlighter {
  private rules: TermHighlightRule[] = [];
  private enabled = true;
  private buf = "";

  constructor(settings?: TermHighlightSettings) {
    if (settings) this.setSettings(settings);
  }

  setSettings(s: TermHighlightSettings) {
    const clean = sanitizeHighlights(s);
    this.enabled = clean.enabled;
    this.rules = resolvedRules(clean);
  }

  feed(chunk: string): string {
    if (!this.enabled) {
      const all = this.buf + chunk;
      this.buf = "";
      return all;
    }
    this.buf += chunk;
    const incompleteAt = this.incompleteEscapeAt(this.buf);
    const ready = incompleteAt >= 0 ? this.buf.slice(0, incompleteAt) : this.buf;
    const restEsc = incompleteAt >= 0 ? this.buf.slice(incompleteAt) : "";
    const { flush, hold } = holdPlainSuffix(ready, this.rules);
    this.buf = hold + restEsc;
    return colorAnsiAware(flush, this.rules);
  }

  flush(): string {
    const left = this.buf;
    this.buf = "";
    if (!this.enabled) return left;
    return colorAnsiAware(left, this.rules);
  }

  private incompleteEscapeAt(s: string): number {
    let i = 0;
    while (i < s.length) {
      if (s[i] === "\x1b") {
        const end = consumeEscape(s, i);
        if (end < 0) return i;
        i = end;
        continue;
      }
      i++;
    }
    return -1;
  }
}

export function previewHighlights(settings: TermHighlightSettings): string {
  const sample = "Interface GigabitEthernet1/0/1 is up, line protocol is down";
  if (!settings.enabled) return sample;
  return colorAnsiAware(sample, resolvedRules(sanitizeHighlights(settings)));
}
