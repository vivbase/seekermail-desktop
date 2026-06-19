// Mail detail header (T041). Renders sender avatar + name + address, recipient
// list (To / Cc, collapsible), full date with timezone, account colour accent,
// and star toggle. TrackerBadge + RemoteImageBar are rendered inside MailBody
// (via SanitizedMail) so the image-reveal DOM ref stays co-located with the
// body element they operate on. Consumes props only — no ipc() calls.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { MailDetail, Recipient } from "@shared/bindings";

import type { AccountColorToken } from "@/lib/accountColor";
import { cn } from "@/lib/cn";
import { SenderAvatar } from "./SenderAvatar";

interface MailHeaderProps {
  mail: MailDetail;
  /** Colour token from the account that owns this mail. */
  colorToken: AccountColorToken;
  isStarred: boolean;
  onToggleStar: () => void;
}

/** Format a Unix timestamp to a long localised date+time string with timezone. */
function formatFullDate(unixSeconds: number): string {
  return new Intl.DateTimeFormat(undefined, {
    weekday: "short",
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    timeZoneName: "short",
  }).format(new Date(unixSeconds * 1000));
}

/** Render a recipient as "Name <email>" or just "email". */
function recipientLabel(r: Recipient): string {
  return r.name ? `${r.name} <${r.email}>` : r.email;
}

interface RecipientListProps {
  label: string;
  recipients: Recipient[];
}

function RecipientList({ label, recipients }: RecipientListProps) {
  const [expanded, setExpanded] = useState(false);

  if (recipients.length === 0) return null;

  const COLLAPSE_AT = 3;
  const visible = expanded ? recipients : recipients.slice(0, COLLAPSE_AT);
  const hidden = recipients.length - COLLAPSE_AT;

  return (
    <div className="flex flex-wrap items-baseline gap-x-1 gap-y-0.5 font-ui text-xs">
      <span className="section-label shrink-0">{label}</span>
      {visible.map((r) => (
        <span key={r.email} className="text-p8">
          {recipientLabel(r)}
        </span>
      ))}
      {!expanded && hidden > 0 && (
        <button
          type="button"
          onClick={() => setExpanded(true)}
          className="rounded-chip bg-p4 px-1.5 py-0.5 font-ui text-[10px] uppercase tracking-wider text-p8 hover:bg-p5 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
        >
          +{hidden} more
        </button>
      )}
      {expanded && hidden > 0 && (
        <button
          type="button"
          onClick={() => setExpanded(false)}
          className="rounded-chip bg-p4 px-1.5 py-0.5 font-ui text-[10px] uppercase tracking-wider text-p8 hover:bg-p5 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
        >
          Show fewer
        </button>
      )}
    </div>
  );
}

export function MailHeader({ mail, isStarred, onToggleStar }: MailHeaderProps) {
  const { t } = useTranslation("reading");

  return (
    <header className="border-b border-divider pb-4">
      {/* Subject — the h1 receives focus on route mount for screen-reader announce */}
      <h1 className="font-display text-2xl italic text-p10">{mail.subject}</h1>

      {/* Sender row */}
      <div className="mt-3 flex items-start gap-3">
        <SenderAvatar email={mail.fromEmail} name={mail.fromName} className="text-sm" />

        <div className="min-w-0 flex-1 space-y-1">
          {/* From */}
          <div className="flex flex-wrap items-baseline gap-x-2">
            <span className="section-label shrink-0">{t("from")}</span>
            <span className="font-ui text-sm font-semibold text-p10">
              {mail.fromName ?? mail.fromEmail}
            </span>
            {mail.fromName && (
              <span className="font-mono text-xs text-p7">&lt;{mail.fromEmail}&gt;</span>
            )}
          </div>

          {/* Recipients */}
          <RecipientList label={t("to")} recipients={mail.to} />
          {mail.cc.length > 0 && <RecipientList label={t("cc")} recipients={mail.cc} />}
        </div>

        {/* Date + star */}
        <div className="flex shrink-0 flex-col items-end gap-2">
          <time
            dateTime={new Date(mail.dateSent * 1000).toISOString()}
            className="font-mono text-xs text-p7"
          >
            {formatFullDate(mail.dateSent)}
          </time>

          <button
            type="button"
            onClick={onToggleStar}
            aria-label={isStarred ? t("unstar") : t("star")}
            aria-pressed={isStarred}
            className={cn(
              "rounded-chip p-1 transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
              isStarred ? "text-amber hover:text-p8" : "text-p7 hover:text-amber",
            )}
          >
            {/* Inline star SVG — no external CDN */}
            <svg
              width="16"
              height="16"
              viewBox="0 0 16 16"
              fill={isStarred ? "currentColor" : "none"}
              stroke="currentColor"
              strokeWidth="1.5"
              aria-hidden="true"
            >
              <path
                strokeLinejoin="round"
                d="M8 1.5 10 6h4.5l-3.5 2.5 1.3 4.5L8 10.5l-4.3 2.5 1.3-4.5L1.5 6H6Z"
              />
            </svg>
          </button>
        </div>
      </div>
    </header>
  );
}
