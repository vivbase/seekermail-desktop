// Pure helpers for the compose "AI Draft" flow (analysis/57 §7). Framework-free
// so they unit-test without a DOM or the Tauri runtime.

/** The sentinel that opens a forwarded-message quote block (see quoteBuilder.ts). */
export const FORWARD_MARKER = "---------- Forwarded message ----------";

/** Byte offset of the quoted block in a compose body, or -1 when there is none. */
export function quoteOffset(body: string): number {
  return body.indexOf(FORWARD_MARKER);
}

/**
 * Place an AI-generated note above the forwarded quote. With no quote (a new
 * mail) the note becomes the body; any text the user had already typed is kept
 * below it rather than discarded.
 */
export function insertNoteAboveQuote(body: string, note: string): string {
  const trimmedNote = note.trimEnd();
  const idx = quoteOffset(body);
  if (idx === -1) {
    const existing = body.trim();
    return existing ? `${trimmedNote}\n\n${existing}` : trimmedNote;
  }
  // Keep the marker and everything below it intact; replace only the leading
  // whitespace the forward seed inserted before the marker.
  const quote = body.slice(idx);
  return `${trimmedNote}\n\n${quote}`;
}

/**
 * A trimmed excerpt of the forwarded message, passed to the backend as prompt
 * context so it never has to look up the mail body. Empty when there is no
 * quote (new-mail mode).
 */
export function forwardedExcerpt(body: string, maxChars = 1200): string {
  const idx = quoteOffset(body);
  if (idx === -1) return "";
  const quote = body.slice(idx).trim();
  return quote.length > maxChars ? quote.slice(0, maxChars) : quote;
}
