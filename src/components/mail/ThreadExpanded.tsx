// Expanded thread sub-list (T040). Lazily fetches all mails in a thread when the
// ThreadGroupCard is expanded. Renders compact ThreadCard rows with indentation.
import { useTranslation } from "react-i18next";

import type { MailSummary } from "@shared/bindings";
import { useMailsInfinite } from "@/ipc/queries/mail";
import { cn } from "@/lib/cn";
import { SkeletonCard } from "./SkeletonCard";
import { ThreadCard } from "./ThreadCard";

interface ThreadExpandedProps {
  threadId: string;
  colorToken: string;
  onArchived?: (threadId: string) => void;
  onDeleted?: (threadId: string) => void;
}

/** Flatten paginated MailSummary pages into a flat array (maps to Thread shape). */
function flattenMails(data: ReturnType<typeof useMailsInfinite>["data"]): MailSummary[] {
  return data ? data.pages.flatMap((p) => p.items) : [];
}

/** Convert a MailSummary to a minimal Thread shape for ThreadCard. */
function mailToThread(mail: MailSummary) {
  return {
    id: mail.id,
    accountId: mail.accountId,
    subject: mail.subject,
    participants: [mail.fromName ?? mail.fromEmail],
    mailCount: 1,
    unreadCount: mail.isRead ? 0 : 1,
    hasAttachments: mail.hasAttachments,
    latestDate: mail.dateSent,
    snippet: mail.snippet,
    isArchived: false,
    isStarred: false,
  };
}

export function ThreadExpanded({
  threadId,
  colorToken,
  onArchived,
  onDeleted,
}: ThreadExpandedProps) {
  const { t } = useTranslation("list");

  const { data, isFetching, hasNextPage, fetchNextPage } = useMailsInfinite({
    threadId,
  });

  const mails = flattenMails(data);

  return (
    <div
      role="list"
      aria-label={t("more_in_thread", { count: mails.length })}
      className="border-b border-divider bg-p2"
    >
      {isFetching && mails.length === 0 && (
        <>
          <SkeletonCard compact />
          <SkeletonCard compact />
        </>
      )}

      {mails.map((mail) => (
        <div key={mail.id} role="listitem" className={cn("ps-6")}>
          <ThreadCard
            thread={mailToThread(mail)}
            colorToken={colorToken}
            senderEmail={mail.fromEmail}
            senderName={mail.fromName}
            onArchived={onArchived}
            onDeleted={onDeleted}
          />
        </div>
      ))}

      {hasNextPage && (
        <button
          type="button"
          onClick={() => void fetchNextPage()}
          className="w-full py-2 font-ui text-xs text-p7 hover:text-p9"
        >
          {t("load_more")}
        </button>
      )}
    </div>
  );
}
