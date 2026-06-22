// Sanitised mail body renderer (T028). Receives the ammonia-cleaned `body_html`
// from IPC and runs a SECOND DOMPurify pass before injection (defence-in-depth,
// 07 §10). Falls back to plain text, truncates oversized bodies, and hosts the
// tracker badge + remote-image bar (T029). Consumes props only — no `invoke`.
import DOMPurify from "dompurify";
import { useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import type { InlineImage, TrackerInfo } from "@shared/bindings";

import { DOMPURIFY_CONFIG, MAX_BODY_BYTES, TRUNCATE_KB } from "@/lib/dompurify-config";
import { applyInlineImages } from "@/lib/mailImages";
import RemoteImageBar from "./RemoteImageBar";
import TrackerBadge from "./TrackerBadge";

interface SanitizedMailProps {
  bodyHtml: string | null;
  bodyText: string | null;
  mailId: string;
  /** Tracker status; when present the badge + image bar render (T029). */
  trackerInfo?: TrackerInfo;
  /** Inline (cid:) images resolved by the route; swapped into the body in place. */
  inlineImages?: InlineImage[];
  /** D1 legal excerpt to highlight in the body (T071 §3.3). Null = none. */
  highlightPhrase?: string | null;
}

/**
 * Wrap the FIRST occurrence of `phrase` in `<mark class="legal-highlight">`
 * (T071 §3.3). XSS-safe by construction: it runs strictly AFTER DOMPurify, the
 * only markup added is the literal `<mark>` wrapper, and the matched text `m`
 * is an existing substring of the already-sanitized HTML. Phrases containing
 * angle brackets are refused outright so a match can never span tag syntax.
 */
export function injectHighlight(html: string, phrase: string | null | undefined): string {
  if (!phrase || phrase.includes("<") || phrase.includes(">")) return html;
  const escaped = phrase.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return html.replace(new RegExp(escaped), (m) => `<mark class="legal-highlight">${m}</mark>`);
}

/** Plain-text variant: split around the first occurrence, no HTML involved. */
function highlightPlainText(text: string, phrase: string | null | undefined) {
  const at = phrase ? text.indexOf(phrase) : -1;
  if (!phrase || at < 0) return text;
  return (
    <>
      {text.slice(0, at)}
      <mark className="legal-highlight">{phrase}</mark>
      {text.slice(at + phrase.length)}
    </>
  );
}

export default function SanitizedMail({
  bodyHtml,
  bodyText,
  mailId,
  trackerInfo,
  inlineImages,
  highlightPhrase,
}: SanitizedMailProps) {
  const { t } = useTranslation();
  const bodyRef = useRef<HTMLDivElement>(null);

  const truncated = !!bodyHtml && bodyHtml.length > MAX_BODY_BYTES;
  const clean = useMemo(() => {
    if (!bodyHtml) return null;
    const source = truncated ? bodyHtml.slice(0, TRUNCATE_KB * 1024) : bodyHtml;
    const sanitized = DOMPurify.sanitize(source, DOMPURIFY_CONFIG) as unknown as string;
    // Legal highlight runs after sanitisation — never on raw input (T071 §3.3).
    return injectHighlight(sanitized, highlightPhrase);
  }, [bodyHtml, truncated, highlightPhrase]);

  const hasRemoteImages = !!bodyHtml && bodyHtml.includes("data-remote-src");

  // Inline (cid:) images carry no privacy cost — swap them in as soon as the
  // body is in the DOM and the resolved bytes arrive (re-runs on either change).
  useEffect(() => {
    applyInlineImages(bodyRef.current, inlineImages);
  }, [clean, inlineImages]);

  return (
    <div className="mx-auto max-w-[680px]">
      <style>{MAIL_BODY_CSS}</style>

      {trackerInfo && <TrackerBadge info={trackerInfo} />}
      {trackerInfo && hasRemoteImages && (
        <RemoteImageBar
          key={mailId}
          mailId={mailId}
          senderEmail={trackerInfo.senderEmail}
          imagesAllowed={trackerInfo.imagesAllowed}
          bodyRef={bodyRef}
        />
      )}

      {clean !== null ? (
        // Only DOMPurify-cleaned strings ever reach dangerouslySetInnerHTML.
        <div
          ref={bodyRef}
          className="seeker-mail-body font-body text-p9"
          dangerouslySetInnerHTML={{ __html: clean }}
        />
      ) : (
        <pre className="seeker-mail-plain whitespace-pre-wrap break-words font-mono text-p9">
          {highlightPlainText(bodyText ?? "", highlightPhrase)}
        </pre>
      )}

      {truncated && (
        <p className="mt-3 rounded-chip bg-p4 px-3 py-2 font-ui text-xs uppercase tracking-wider text-p8">
          {t("mail_body_truncated", { sizeKb: TRUNCATE_KB })}
        </p>
      )}
    </div>
  );
}

// Body styling via design tokens (no bare hex). Logical properties keep RTL safe.
//
// IMPORTANT: HTML marketing emails nest <table>s purely for layout (spacer
// cells, image cells, column cells). We must NOT impose a border on every cell —
// doing so turns each layout cell into an empty box and shreds the design. We
// also keep our own rules low-specificity (no `!important`) so the sender's
// preserved inline styles / presentational attributes win, and contain wide
// tables instead of letting them overflow the reading column.
const MAIL_BODY_CSS = `
/* Reading text size (analysis 25, Layer 2): the 14px base is multiplied by
   --reading-scale (default 1), set on <html> by lib/readingScale.ts. This scales
   the email body only; the app chrome is unaffected. */
.seeker-mail-body { font-size: calc(14px * var(--reading-scale, 1)); line-height: 1.6; word-break: break-word; overflow-x: auto; }
.seeker-mail-plain { font-size: calc(14px * var(--reading-scale, 1)); line-height: 1.6; }
.seeker-mail-body a { color: var(--terra); text-decoration: underline; }
.seeker-mail-body img { max-width: 100%; height: auto; }
/* Remote images are emptied at ingest (src="") and restored on demand by the
   image bar; inline images carry an unresolved cid: src until the bytes arrive.
   Hide both until their real src is swapped in, so no broken frame flashes. */
.seeker-mail-body img[src=""],
.seeker-mail-body img:not([src]),
.seeker-mail-body img[src^="cid:" i] { display: none; }
.seeker-mail-body blockquote {
  border-inline-start: 4px solid var(--p5);
  padding-inline-start: 12px;
  margin-inline-start: 0;
  color: var(--p8);
}
/* Contain layout tables; never force a border on every cell. If the email wants
   visible borders it carries them via its own attributes / inline style. */
.seeker-mail-body table { border-collapse: collapse; max-width: 100%; }
.seeker-mail-body td, .seeker-mail-body th { vertical-align: top; }
.seeker-mail-body pre, .seeker-mail-body code { font-family: var(--fm); }
.seeker-mail-body pre { white-space: pre-wrap; word-break: break-word; }
mark.legal-highlight {
  background: color-mix(in srgb, var(--terra) 18%, transparent);
  color: inherit;
  border-radius: 2px;
}
`;
