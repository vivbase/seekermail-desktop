// Single search result row (T034). Renders subject, sender, relative date,
// score-label chip, and highlight fragments. Highlights are split on <mark>/</mark>
// boundaries and rendered as React elements — no dangerouslySetInnerHTML needed.
import { useTranslation } from "react-i18next";

import type { AttachmentHit, ScoreLabel, SearchResult } from "@shared/bindings";
import { cn } from "@/lib/cn";

// ── Relative date formatter ───────────────────────────────────────────────────

const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });

function relativeDateSent(epochSec: number): string {
  const diffSec = epochSec - Math.floor(Date.now() / 1000);
  const diffMin = Math.round(diffSec / 60);
  const diffHr = Math.round(diffSec / 3600);
  const diffDay = Math.round(diffSec / 86400);
  if (Math.abs(diffMin) < 60) return rtf.format(diffMin, "minute");
  if (Math.abs(diffHr) < 24) return rtf.format(diffHr, "hour");
  if (Math.abs(diffDay) < 30) return rtf.format(diffDay, "day");
  // Fall back to a locale date string for older mail.
  return new Date(epochSec * 1000).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

// ── Highlight renderer ────────────────────────────────────────────────────────

/** Split a highlight fragment string on <mark>…</mark> and render <mark> elements. */
function HighlightFragment({ text }: { text: string }) {
  // Pattern: split on opening and closing tags, keeping the tag boundaries.
  const parts = text.split(/(<mark>|<\/mark>)/);
  const nodes: React.ReactNode[] = [];
  let inMark = false;
  let key = 0;

  for (const part of parts) {
    if (part === "<mark>") {
      inMark = true;
    } else if (part === "</mark>") {
      inMark = false;
    } else if (part) {
      if (inMark) {
        nodes.push(
          <mark key={key++} className="bg-amber/30 rounded-sm not-italic text-p10">
            {part}
          </mark>,
        );
      } else {
        nodes.push(<span key={key++}>{part}</span>);
      }
    }
  }

  return <>{nodes}</>;
}

// ── Score chip ────────────────────────────────────────────────────────────────

const SCORE_CLASS: Record<ScoreLabel, string> = {
  high: "text-green bg-green/10",
  mid: "text-slate bg-slate/10",
  low: "text-p7 bg-p5",
};

interface ScoreChipProps {
  label: ScoreLabel;
}

function ScoreChip({ label }: ScoreChipProps) {
  const { t } = useTranslation("search");
  const labelKey = label === "high" ? "score_high" : label === "mid" ? "score_mid" : "score_low";
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center rounded-chip px-1.5 py-0.5 font-ui text-[9px] font-semibold uppercase leading-none tracking-wider",
        SCORE_CLASS[label],
      )}
    >
      {t(labelKey)}
    </span>
  );
}

// ── Attachment badge icon (inline paperclip — no icon library, T110 §6) ─────────

function PaperclipIcon() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      className="shrink-0 text-p7"
      aria-hidden="true"
    >
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        d="M21.44 11.05l-9.19 9.19a5 5 0 0 1-7.07-7.07l9.19-9.19a3 3 0 0 1 4.24 4.24l-9.2 9.19a1 1 0 0 1-1.41-1.41l8.49-8.49"
      />
    </svg>
  );
}

function labelForScore(score: number): ScoreLabel {
  return score >= 0.7 ? "high" : score >= 0.4 ? "mid" : "low";
}

// ── SearchResultCard ──────────────────────────────────────────────────────────

export type SearchResultCardProps =
  | { source?: "mail"; result: SearchResult; selected: boolean; onClick: () => void }
  | { source: "attachment"; hit: AttachmentHit; selected: boolean; onClick: () => void };

export function SearchResultCard(props: SearchResultCardProps) {
  if ("hit" in props) {
    return (
      <AttachmentResultCard hit={props.hit} selected={props.selected} onClick={props.onClick} />
    );
  }
  const { result, selected, onClick } = props;
  const { subject, fromName, fromEmail, dateSent, highlights, scoreLabel } = result;
  const displaySender = fromName ?? fromEmail;
  const relDate = relativeDateSent(dateSent);

  return (
    <button
      type="button"
      onClick={onClick}
      aria-selected={selected}
      className={cn(
        "flex w-full flex-col gap-1 rounded-chip px-4 py-3 text-start transition-colors",
        "hover:bg-p4",
        selected && "bg-p4 ring-1 ring-inset ring-divider",
      )}
    >
      {/* Row 1: subject + score chip */}
      <div className="flex items-start justify-between gap-3">
        <span
          className={cn(
            "min-w-0 truncate font-body text-sm leading-snug",
            selected ? "font-semibold text-p10" : "text-p10",
          )}
        >
          {subject}
        </span>
        <ScoreChip label={scoreLabel} />
      </div>

      {/* Row 2: sender + date */}
      <div className="flex items-center justify-between gap-3">
        <span className="min-w-0 truncate font-ui text-xs text-p8">{displaySender}</span>
        <span className="shrink-0 font-mono text-xs text-p7">{relDate}</span>
      </div>

      {/* Highlights */}
      {highlights.length > 0 && (
        <p className="line-clamp-2 font-body text-xs leading-relaxed text-p7">
          {highlights.map((h, i) => (
            <span key={i}>
              {i > 0 && <span className="mx-1 text-p5">·</span>}
              <HighlightFragment text={h} />
            </span>
          ))}
        </p>
      )}
    </button>
  );
}

// ── Attachment-origin result card (T110) ────────────────────────────────────────

function AttachmentResultCard({
  hit,
  selected,
  onClick,
}: {
  hit: AttachmentHit;
  selected: boolean;
  onClick: () => void;
}) {
  const { t } = useTranslation("search");
  const relDate = relativeDateSent(hit.mailDateSent);

  return (
    <button
      type="button"
      onClick={onClick}
      aria-selected={selected}
      className={cn(
        "flex w-full flex-col gap-1 rounded-chip px-4 py-3 text-start transition-colors",
        "hover:bg-p4",
        selected && "bg-p4 ring-1 ring-inset ring-divider",
      )}
    >
      {/* Row 1: owning mail subject + score chip */}
      <div className="flex items-start justify-between gap-3">
        <span className="min-w-0 truncate font-body text-sm leading-snug text-p10">
          {hit.mailSubject}
        </span>
        <ScoreChip label={labelForScore(hit.score)} />
      </div>

      {/* Row 2: "in attachment" badge + filename (mono, var(--fm)) */}
      <div className="flex items-center gap-1.5">
        <PaperclipIcon />
        <span className="shrink-0 font-ui text-[10px] uppercase tracking-wider text-p8">
          {t("search_in_attachment")}
        </span>
        <span className="min-w-0 truncate font-mono text-xs text-p9" title={hit.filename}>
          {hit.filename}
        </span>
      </div>

      {/* Row 3: excerpt with <mark> highlights */}
      {hit.excerpt && (
        <p className="line-clamp-2 font-body text-xs leading-relaxed text-p7">
          <HighlightFragment text={hit.excerpt} />
        </p>
      )}

      {/* Row 4: sender + date */}
      <div className="flex items-center justify-between gap-3">
        <span className="min-w-0 truncate font-ui text-xs text-p8">{hit.mailFromEmail}</span>
        <span className="shrink-0 font-mono text-xs text-p7">{relDate}</span>
      </div>
    </button>
  );
}
