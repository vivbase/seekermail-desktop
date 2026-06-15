// Inline-style CSS scrubber — defence-in-depth for HTML email rendering.
//
// HTML marketing emails carry their layout almost entirely in inline `style`
// attributes (widths, padding, alignment, colour, font sizing). We must keep
// those so the message renders the way the sender intended — but a raw `style`
// value is an XSS / privacy surface:
//   • `url(...)` re-introduces the remote requests the image blocker stripped
//     (re-enabling tracking pixels and exfiltration).
//   • `position: fixed/absolute` can overlay the app chrome for phishing.
//   • `expression()` / `javascript:` / `@import` are classic CSS injection.
//
// `scrubInlineStyle` keeps ONLY an allowlist of inert presentational properties
// and drops any declaration whose value looks dangerous. It is a pure
// string→string function (no DOM), so it runs unchanged inside the DOMPurify
// `uponSanitizeAttribute` hook and is trivially unit-testable. The Rust ingest
// pass mirrors this policy (`sanitize::scrub_style`) so persisted `body_html`
// is already clean; this is the second, never-trust-storage pass (07 §10).

/** Inert CSS properties that cannot load remote resources or escape the pane. */
export const SAFE_CSS_PROPERTIES: ReadonlySet<string> = new Set([
  // Colour & typography
  "color",
  "background-color",
  "font",
  "font-family",
  "font-size",
  "font-weight",
  "font-style",
  "font-variant",
  "line-height",
  "letter-spacing",
  "word-spacing",
  "text-align",
  "text-decoration",
  "text-transform",
  "text-indent",
  "text-overflow",
  "white-space",
  "word-break",
  "overflow-wrap",
  "word-wrap",
  "direction",
  "unicode-bidi",
  "vertical-align",
  // Box model
  "width",
  "min-width",
  "max-width",
  "height",
  "min-height",
  "max-height",
  "padding",
  "padding-top",
  "padding-right",
  "padding-bottom",
  "padding-left",
  "padding-block",
  "padding-block-start",
  "padding-block-end",
  "padding-inline",
  "padding-inline-start",
  "padding-inline-end",
  "margin",
  "margin-top",
  "margin-right",
  "margin-bottom",
  "margin-left",
  "margin-block",
  "margin-block-start",
  "margin-block-end",
  "margin-inline",
  "margin-inline-start",
  "margin-inline-end",
  // Borders
  "border",
  "border-top",
  "border-right",
  "border-bottom",
  "border-left",
  "border-width",
  "border-style",
  "border-color",
  "border-top-width",
  "border-top-style",
  "border-top-color",
  "border-right-width",
  "border-right-style",
  "border-right-color",
  "border-bottom-width",
  "border-bottom-style",
  "border-bottom-color",
  "border-left-width",
  "border-left-style",
  "border-left-color",
  "border-radius",
  "border-top-left-radius",
  "border-top-right-radius",
  "border-bottom-left-radius",
  "border-bottom-right-radius",
  "border-collapse",
  "border-spacing",
  // Table / list / display — note: positioning props are intentionally absent.
  "display",
  "box-sizing",
  "table-layout",
  "empty-cells",
  "caption-side",
  "list-style-type",
  "list-style-position",
]);

// Any declaration value matching this is dropped outright: remote loads,
// script execution, stylesheet imports, or attempts to break out via markup /
// CSS escapes / comments / HTML entities.
const UNSAFE_VALUE = /url\s*\(|expression\s*\(|javascript:|vbscript:|@import|[<>\\]|\/\*|&#/i;

// Hard caps so a pathological style string can't blow up the parser.
const MAX_DECLARATIONS = 64;
const MAX_VALUE_LENGTH = 256;

/**
 * Reduce a raw inline-`style` value to a safe, presentational-only subset.
 * Returns "" when nothing survives (callers should then drop the attribute).
 */
export function scrubInlineStyle(raw: string | null | undefined): string {
  if (!raw) return "";
  const out: string[] = [];
  for (const decl of raw.split(";")) {
    if (out.length >= MAX_DECLARATIONS) break;
    const idx = decl.indexOf(":");
    if (idx < 0) continue;
    const prop = decl.slice(0, idx).trim().toLowerCase();
    const value = decl.slice(idx + 1).trim();
    if (!prop || !value || value.length > MAX_VALUE_LENGTH) continue;
    if (!SAFE_CSS_PROPERTIES.has(prop)) continue;
    if (UNSAFE_VALUE.test(value)) continue;
    out.push(`${prop}: ${value}`);
  }
  return out.join("; ");
}
