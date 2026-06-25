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

/** How a grounded agent reply was assembled by the Mailbox Context Engine —
 *  mirrors the Rust `RetrievalReport` (camelCase). Embedded in an agent text
 *  message's `content` so the UI can show an honest "what was searched" chip
 *  instead of a silent empty answer (analysis/54 §3.4). */
export type RetrievalState = {
  /** `false` when the semantic index/embedder could not run at all. */
  semanticAvailable: boolean;
  /** Semantic hits used. */
  semanticHits: number;
  /** Recent-mail (temporal) hits used. */
  temporalHits: number;
  /** Computed facts used (counts, top senders). */
  aggregateFacts: number;
  /** Precomputed thread summaries used (memory leg). */
  memoryHits: number;
  /** Mails already embedded into the semantic index. */
  indexedMails: number;
  /** Stored, non-deleted mails — the coverage denominator. */
  totalMails: number;
};

/** Read the optional `retrieval` state out of an agent message body; `null`
 *  when absent (fallback notes, older messages) or on any parse failure. */
export function parseRetrievalState(content: string): RetrievalState | null {
  try {
    const parsed = JSON.parse(content) as { retrieval?: Partial<RetrievalState> };
    const r = parsed.retrieval;
    if (!r || typeof r !== "object") return null;
    return {
      semanticAvailable: Boolean(r.semanticAvailable),
      semanticHits: Number(r.semanticHits ?? 0),
      temporalHits: Number(r.temporalHits ?? 0),
      aggregateFacts: Number(r.aggregateFacts ?? 0),
      memoryHits: Number(r.memoryHits ?? 0),
      indexedMails: Number(r.indexedMails ?? 0),
      totalMails: Number(r.totalMails ?? 0),
    };
  } catch {
    return null;
  }
}
