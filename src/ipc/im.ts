// Hand-written mirror of the Rust `ImMessage` DTO (T092, `src-tauri/src/types.rs`
// Module I) until `pnpm gen:types` emits it into `@shared/bindings`. Field shapes
// follow the generated conventions: camelCase, `| null` optionals. Once the
// command surface is exported, delete this file and import from there instead.

export type ImSenderType = "human" | "agent" | "system";
export type ImMessageType = "text" | "query_card" | "card_reply" | "status";
export type ImMessageStatus = "pending" | "answered" | "skipped" | "resolved";

/** One Agent-IM (TEAM) channel message. `content` is a JSON string whose shape
 *  depends on `messageType` (text/status → `{ "text": "…" }`). */
export type ImMessage = {
  id: string;
  channelId: string;
  senderType: ImSenderType;
  senderId: string;
  messageType: ImMessageType;
  content: string;
  linkedEmailId: string | null;
  status: ImMessageStatus;
  createdAt: number;
  readAt: number | null;
};

/** The single shared channel — there are no private chats (root CLAUDE.md). */
export const MAIN_CHANNEL = "main";

/** Build the JSON content string for a plain text / status message. */
export function textContent(text: string): string {
  return JSON.stringify({ text });
}

/** Read the `text` field out of a text/status message body; "" on any failure. */
export function parseMessageText(content: string): string {
  try {
    const parsed = JSON.parse(content) as { text?: unknown };
    return typeof parsed.text === "string" ? parsed.text : "";
  } catch {
    return "";
  }
}
