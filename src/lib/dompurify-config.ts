// DOMPurify allowlist for the second sanitisation pass (T028, 07 §10), kept in one
// place rather than scattered through components. The tag/attr allowlist is aligned
// with the Rust ammonia policy (T027); `data-remote-src` is preserved so the
// "load images" affordance can swap it back into `src`.
//
// Presentational table attributes (`align`, `valign`, `bgcolor`, `width`,
// `height`, `cellpadding`, `cellspacing`, `border`) and a CSS-scrubbed `style`
// attribute are allowed so HTML mail keeps its intended layout. `style` is never
// trusted verbatim: the `uponSanitizeAttribute` hook below runs every value
// through `scrubInlineStyle`, which strips `url()`, `position`, `expression()`,
// and other CSS attack vectors.
import DOMPurify from "dompurify";
import type { Config } from "dompurify";

import { scrubInlineStyle } from "./cssScrub";

export const DOMPURIFY_CONFIG: Config = {
  ALLOWED_TAGS: [
    "p",
    "span",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "br",
    "hr",
    "a",
    "img",
    "ul",
    "ol",
    "li",
    "table",
    "thead",
    "tbody",
    "tr",
    "td",
    "th",
    "blockquote",
    "pre",
    "code",
    "em",
    "strong",
    "b",
    "i",
    "u",
    "sup",
    "sub",
    "figure",
    "figcaption",
  ],
  ALLOWED_ATTR: [
    "href",
    "title",
    "rel",
    "src",
    "alt",
    "width",
    "height",
    "colspan",
    "rowspan",
    "data-remote-src",
    // Presentational layout attributes used by HTML mail (inert — no script or
    // URL vectors). Mirrored in the Rust ammonia allowlist (T027).
    "align",
    "valign",
    "bgcolor",
    "cellpadding",
    "cellspacing",
    "border",
    // Scrubbed to a safe property subset by the hook below before injection.
    "style",
  ],
  FORCE_BODY: true,
  // Never allow these even if present.
  FORBID_TAGS: ["script", "style", "iframe", "object", "embed", "form", "input", "button"],
  RETURN_DOM: false,
  RETURN_DOM_FRAGMENT: false,
};

/** Bodies above this size are truncated before injection (F_B1 §6). */
export const MAX_BODY_BYTES = 5_000_000;
/** Size of the safe prefix we still render when truncating (KB). */
export const TRUNCATE_KB = 500;

let hooksInstalled = false;

/**
 * Register the one-time `style`-scrubbing hook on the shared DOMPurify
 * singleton. Idempotent — safe to call from every sanitisation site. The hook
 * only ever touches `style` attributes, so it cannot affect any other DOMPurify
 * consumer beyond making their CSS safer.
 */
export function installSanitizerHooks(): void {
  if (hooksInstalled) return;
  hooksInstalled = true;
  DOMPurify.addHook("uponSanitizeAttribute", (_node, data) => {
    if (data.attrName !== "style") return;
    const cleaned = scrubInlineStyle(data.attrValue);
    if (cleaned) {
      data.attrValue = cleaned;
    } else {
      data.keepAttr = false;
    }
  });
}

// Install immediately so any importer of DOMPURIFY_CONFIG gets the scrub hook
// without having to remember to call it (the guard keeps this single-shot).
installSanitizerHooks();
