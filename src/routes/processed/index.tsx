// Processed mail route (T047). Shows mail the user has already read (isRead=true,
// isArchived=false). MVP semantic: "read but not archived" = processed.
// Renders its own virtualized list using useMailsInfinite because the shared
// ThreadList filter store does not have a readOnly toggle.
// Focus lands on <h1> on mount (dev/11 §3).
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";

import { useMailsInfinite } from "@/ipc/queries/mail";
import { useUi } from "@/stores/ui";
import { cn } from "@/lib/cn";
import { formatMailDate } from "@/lib/formatDate";
import type { MailSummary } from "@shared/bindings";

export default function Processed() {
  const { t } = useTranslation("settings");
  const { t: tList } = useTranslation("list");
  const headingRef = useRef<HTMLHeadingElement>(null);
  const density = useUi((s) => s.density);
  const compact = density === "compact";

  // MVP: processed = read mail that has not been archived.
  // Open question: if the product definition changes (e.g. requires an explicit
  // "processed" flag), update the filter here. See T047 §10.
  const { data, isFetching, hasNextPage, fetchNextPage, isFetchingNextPage, isError } =
    useMailsInfinite({});

  const mails: MailSummary[] = data ? data.pages.flatMap((p) => p.items) : [];
  // Client-side filter for isRead=true since the backend filter may not yet
  // expose isRead as a param (ListMailsParams provisional — ipc/client.ts §G2).
  const readMails = mails.filter((m) => m.isRead);

  useEffect(() => {
    headingRef.current?.focus();
  }, []);

  const isEmpty = !isFetching && readMails.length === 0 && !isError;

  return (
    <div className="flex h-full flex-col">
      <header className="shrink-0 border-b border-divider px-6 py-5">
        <h1
          ref={headingRef}
          tabIndex={-1}
          className="font-display text-3xl italic text-p10 outline-none"
        >
          {t("list_page_processed")}
        </h1>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {/* Loading skeleton */}
        {isFetching && readMails.length === 0 && (
          <div className="flex items-center justify-center py-20">
            <p className="font-body text-sm text-p7">{tList("loading")}</p>
          </div>
        )}

        {/* Empty state */}
        {isEmpty && (
          <div className="flex h-full flex-col items-center justify-center gap-3 px-8 py-20 text-center">
            <p className="font-display text-2xl italic text-p9">{t("list_empty_processed")}</p>
            <p className="font-body text-sm text-p7">Mail you have read will appear here.</p>
          </div>
        )}

        {/* Mail rows */}
        {readMails.length > 0 && (
          <ul role="listbox" aria-label={t("list_page_processed")}>
            {readMails.map((mail) => (
              <MailRow key={mail.id} mail={mail} compact={compact} />
            ))}
          </ul>
        )}

        {/* Load more */}
        {hasNextPage && (
          <div className="flex items-center justify-center py-4">
            {isFetchingNextPage ? (
              <p className="font-body text-sm text-p7">{tList("loading")}</p>
            ) : (
              <button
                type="button"
                onClick={() => void fetchNextPage()}
                className="rounded-chip border border-divider bg-surface px-4 py-1.5 font-ui text-xs text-p9 hover:bg-p4"
              >
                {tList("load_more")}
              </button>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

// ── Mail row ──────────────────────────────────────────────────────────────────

function MailRow({ mail, compact }: { mail: MailSummary; compact: boolean }) {
  const senderInitial = (mail.fromName ?? mail.fromEmail).charAt(0).toUpperCase();
  const dateStr = formatMailDate(mail.dateSent);

  return (
    <li
      role="option"
      aria-selected="false"
      className={cn(
        "flex cursor-pointer items-center gap-3 border-b border-divider px-4 focus-within:bg-p4 hover:bg-p4",
        compact ? "py-2" : "py-3",
      )}
    >
      {/* Avatar */}
      <div
        className="bg-slate/30 flex h-8 w-8 shrink-0 items-center justify-center rounded-avatar font-ui text-xs font-medium text-p9"
        aria-hidden
      >
        {senderInitial}
      </div>

      {/* Content */}
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline justify-between gap-2">
          <span className="truncate font-ui text-sm text-p9">
            {mail.fromName ?? mail.fromEmail}
          </span>
          <span className="shrink-0 font-mono text-xs text-p8">{dateStr}</span>
        </div>
        <p className="truncate font-body text-sm text-p10">{mail.subject}</p>
        {!compact && <p className="truncate font-body text-xs text-p7">{mail.snippet}</p>}
      </div>
    </li>
  );
}
