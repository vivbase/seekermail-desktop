// Plain-text ↔ HTML helpers for the rich-text compose editor (T044, F_G4 §4.4).
// Pure functions — no React, no live-DOM dependency — so they are unit-testable
// and safe to run anywhere. The editor keeps a plain-text mirror (`body`) beside
// the HTML (`bodyHtml`); these helpers bridge the two for seeding, sending, and
// draft persistence.

/** Escape the HTML-significant characters so plain text renders literally. */
export function escapeHtml(input: string): string {
  return input
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

/**
 * Convert a plain-text body (e.g. a reply/forward quote seed or an AI draft)
 * into editor-ready HTML. Newlines become `<br>` so spacing survives the
 * round-trip into the contentEditable surface. Returns "" for empty input.
 */
export function plainTextToHtml(input: string): string {
  if (!input) return "";
  return escapeHtml(input).replace(/\r?\n/g, "<br>");
}

/**
 * Best-effort HTML → plain-text reduction. The live editor derives its plain
 * mirror from `element.innerText`; this fallback is used where no rendered
 * element is available (and in test environments where `innerText` is absent).
 * Block-level closers and `<br>` collapse to newlines; tags are stripped and the
 * common named/numeric entities decoded.
 */
export function htmlToPlainText(html: string): string {
  if (!html) return "";
  return html
    .replace(/<\s*br\s*\/?>/gi, "\n")
    .replace(/<\/\s*(?:div|p|li|h[1-6]|blockquote|tr)\s*>/gi, "\n")
    .replace(/<[^>]+>/g, "")
    .replace(/&nbsp;/gi, " ")
    .replace(/&lt;/gi, "<")
    .replace(/&gt;/gi, ">")
    .replace(/&quot;/gi, '"')
    .replace(/&#39;/gi, "'")
    .replace(/&amp;/gi, "&")
    .replace(/\n{3,}/g, "\n\n")
    .replace(/[ \t]+\n/g, "\n")
    .trim();
}

/**
 * True when HTML carries no visible content — empty string, whitespace, or
 * structural-only markup like `<div><br></div>`. Used to decide whether to send
 * a `text/html` MIME part at all (a blank HTML body is sent as `null`).
 */
export function isHtmlBlank(html: string): boolean {
  if (!html) return true;
  const stripped = html
    .replace(/<\s*br\s*\/?>/gi, "")
    .replace(/<[^>]+>/g, "")
    .replace(/&nbsp;/gi, "")
    .replace(/\s+/g, "");
  return stripped.length === 0;
}
