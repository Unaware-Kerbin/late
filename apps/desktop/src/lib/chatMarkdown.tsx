import type { ReactNode } from "react";

const ROLE_LINE = /^(assistant|user|system|tool|model|human|ai)\s*:?\s*$/i;
const ROLE_PREFIX = /^(assistant|user|system|tool|model|human|ai)\s*:\s*/i;

/** Light spacing fixes outside fenced code — models often glue words to hostnames/IPs. */
export function polishChatText(src: string): string {
  if (!src) return "";
  const normalized = src.replace(/\r\n/g, "\n");
  const parts = normalized.split(/(```[\s\S]*?```)/g);
  return parts
    .map((part, idx) => {
      if (idx % 2 === 1 || part.startsWith("```")) return part;
      return polishProse(part);
    })
    .join("");
}

function polishProse(text: string): string {
  let t = text.replace(/^\uFEFF/, "");
  // Drop a leading raw role dump on its own line or as "assistant: …"
  const lines = t.split("\n");
  while (lines.length && (!lines[0]!.trim() || ROLE_LINE.test(lines[0]!.trim()))) {
    lines.shift();
  }
  if (lines.length) lines[0] = lines[0]!.replace(ROLE_PREFIX, "");
  t = lines.join("\n");

  t = t
    // "from6200F" / "vlan10" — lowercase word glued to digits (keeps IPv4 / SHA256)
    .replace(/([a-z]{2,})(\d)/g, "$1 $2")
    .replace(/([A-Za-z])\(/g, "$1 (")
    // Missing space after sentence punctuation before a capital
    .replace(/([.!?])([A-Z])/g, "$1 $2")
    // Em/en dash glued to words
    .replace(/(\S)—(\S)/g, "$1 — $2")
    .replace(/(\S)–(\S)/g, "$1 – $2")
    // Collapse accidental multi-spaces (keep newlines)
    .replace(/[^\S\n]{2,}/g, " ");

  // Soft paragraph breaks for dense single-line run-ons with multiple sentences
  if (!t.includes("\n") && t.length > 140) {
    t = t.replace(/([.!?])\s+(?=[A-Z“"])/g, "$1\n\n");
  } else if (t.length > 160) {
    t = t
      .split("\n")
      .map((line) =>
        line.length > 140 && /[.!?]\s+[A-Z]/.test(line)
          ? line.replace(/([.!?])\s+(?=[A-Z“"])/g, "$1\n\n")
          : line,
      )
      .join("\n");
  }

  return t.trim();
}

function inlineParts(text: string, keyBase: string): ReactNode[] {
  const out: ReactNode[] = [];
  const re = /(`[^`]+`|\*\*[^*]+\*\*)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  let i = 0;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) out.push(text.slice(last, m.index));
    const tok = m[0];
    if (tok.startsWith("`")) {
      out.push(
        <code key={`${keyBase}-c${i++}`} className="md-inline-code">
          {tok.slice(1, -1)}
        </code>,
      );
    } else {
      out.push(
        <strong key={`${keyBase}-b${i++}`}>{tok.slice(2, -2)}</strong>,
      );
    }
    last = m.index + tok.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

/**
 * Lightweight markdown for agent replies: fenced code, lists, paragraphs,
 * inline code, bold. Escapes nothing as HTML — all nodes are React text.
 */
export function renderChatMarkdown(src: string): ReactNode {
  if (!src) return null;
  const lines = polishChatText(src).split("\n");
  const blocks: ReactNode[] = [];
  let i = 0;
  let bi = 0;

  while (i < lines.length) {
    const line = lines[i]!;

    const fence = line.match(/^```(\w*)\s*$/);
    if (fence) {
      const lang = fence[1] || "";
      const body: string[] = [];
      i += 1;
      while (i < lines.length && !/^```\s*$/.test(lines[i]!)) {
        body.push(lines[i]!);
        i += 1;
      }
      if (i < lines.length) i += 1;
      blocks.push(
        <pre key={`md-pre-${bi++}`} className="md-code" data-lang={lang || undefined}>
          <code>{body.join("\n")}</code>
        </pre>,
      );
      continue;
    }

    if (/^\s*[-*]\s+/.test(line)) {
      const items: ReactNode[] = [];
      while (i < lines.length && /^\s*[-*]\s+/.test(lines[i]!)) {
        const item = lines[i]!.replace(/^\s*[-*]\s+/, "");
        items.push(<li key={`md-li-${bi}-${items.length}`}>{inlineParts(item, `li-${bi}-${items.length}`)}</li>);
        i += 1;
      }
      blocks.push(
        <ul key={`md-ul-${bi++}`} className="md-list">
          {items}
        </ul>,
      );
      continue;
    }

    if (/^\s*\d+\.\s+/.test(line)) {
      const items: ReactNode[] = [];
      while (i < lines.length && /^\s*\d+\.\s+/.test(lines[i]!)) {
        const item = lines[i]!.replace(/^\s*\d+\.\s+/, "");
        items.push(<li key={`md-oli-${bi}-${items.length}`}>{inlineParts(item, `oli-${bi}-${items.length}`)}</li>);
        i += 1;
      }
      blocks.push(
        <ol key={`md-ol-${bi++}`} className="md-list">
          {items}
        </ol>,
      );
      continue;
    }

    if (/^#{1,3}\s+\S/.test(line)) {
      const level = (line.match(/^#+/)?.[0].length ?? 1) as 1 | 2 | 3;
      const text = line.replace(/^#{1,3}\s+/, "");
      const Tag = (`h${Math.min(3, level) + 2}` as "h3" | "h4" | "h5");
      blocks.push(
        <Tag key={`md-h-${bi++}`} className="md-heading">
          {inlineParts(text, `h-${bi}`)}
        </Tag>,
      );
      i += 1;
      continue;
    }

    if (!line.trim()) {
      i += 1;
      continue;
    }

    const para: string[] = [];
    while (
      i < lines.length &&
      lines[i]!.trim() &&
      !/^```/.test(lines[i]!) &&
      !/^\s*[-*]\s+/.test(lines[i]!) &&
      !/^\s*\d+\.\s+/.test(lines[i]!) &&
      !/^#{1,3}\s+\S/.test(lines[i]!)
    ) {
      para.push(lines[i]!);
      i += 1;
    }
    blocks.push(
      <p key={`md-p-${bi++}`} className="md-p">
        {inlineParts(para.join("\n"), `p-${bi}`)}
      </p>,
    );
  }

  return <div className="md-body">{blocks}</div>;
}
