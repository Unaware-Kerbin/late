import type { ChatMsg } from "../types";

/** Bound in-memory chat rows so multi-turn Agent + MCP speaker spam cannot OOM the renderer. */
export const MAX_CHAT_MESSAGES = 80;
/** Per-message body cap (speaker / tool / system rows can otherwise retain multi‑MB blobs). */
export const MAX_CHAT_CONTENT_CHARS = 48_000;
/** Base64 logos repeat on every speaker turn; drop duplicates and absurdly large ones. */
export const MAX_LOGO_DATA_URL_CHARS = 64_000;

export function capChatContent(text: string, maxChars = MAX_CHAT_CONTENT_CHARS): string {
  if (typeof text !== "string" || text.length <= maxChars) return text ?? "";
  const omitted = text.length - maxChars;
  return `${text.slice(0, maxChars)}\n…[truncated ${omitted} chars]`;
}

export function capChatMessage(msg: ChatMsg): ChatMsg {
  const content = capChatContent(msg.content);
  const logo = msg.logoDataUrl;
  if (logo && logo.length > MAX_LOGO_DATA_URL_CHARS) {
    const { logoDataUrl: _drop, ...rest } = msg;
    return { ...rest, content };
  }
  if (content === msg.content) return msg;
  return { ...msg, content };
}

/** Keep logoDataUrl only on the newest message that carries each distinct URL. */
export function dedupeChatLogos(msgs: ChatMsg[]): ChatMsg[] {
  const seen = new Set<string>();
  const out = msgs.slice();
  for (let i = out.length - 1; i >= 0; i--) {
    const logo = out[i].logoDataUrl;
    if (!logo) continue;
    if (seen.has(logo)) {
      const { logoDataUrl: _drop, ...rest } = out[i];
      out[i] = rest;
    } else {
      seen.add(logo);
    }
  }
  return out;
}

export function trimChatMessages(msgs: ChatMsg[]): ChatMsg[] {
  const capped = msgs.map(capChatMessage);
  const deduped = dedupeChatLogos(capped);
  if (deduped.length <= MAX_CHAT_MESSAGES) return deduped;
  return deduped.slice(deduped.length - MAX_CHAT_MESSAGES);
}
