// Virtualized infinite thread stream (T037, T040). The primary L0 view component.
// Renders ThreadCard rows (flat mode) or ThreadGroupCard rows (fold mode).
// Uses TanStack Virtual for a windowed DOM (≤ 60 nodes at steady-state).
import { forwardRef, useCallback, useEffect, useImperativeHandle, useMemo, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useTranslation } from "react-i18next";

import type { MailSummary, Thread } from "@shared/bindings";
import { useMailsInfinite, useThreadsInfinite } from "@/ipc/queries/mail";
import { useAccounts } from "@/ipc/queries/accounts";
import { useUi } from "@/stores/ui";
import { useSelection } from "@/stores/selection";
import { SkeletonCard } from "./SkeletonCard";
import { ThreadCard } from "./ThreadCard";
import { ThreadGroupCard } from "./ThreadGroupCard";
import { ThreadExpanded } from "./ThreadExpanded";

// ── Types ─────────────────────────────────────────────────────────────────────

interface ThreadListProps {
  /** Limit results to a specific account. Null = all accounts. */
  accountId: string | null;
  /** Limit results to a mailbox folder (e.g. "SENT"). Null/undefined = all folders. */
  folder?: string | null;
  /** Called when a card is archived so the parent can show UndoToast. */
  onArchived?: (threadId: string) => void;
  /** Called when a card is deleted so the parent can show UndoToast. */
  onDeleted?: (threadId: string) => void;
}

export interface ThreadListHandle {
  /** Scroll the virtual list to a specific item index. */
  scrollToIndex: (index: number) => void;
}

// ── Helper: flatten pages ─────────────────────────────────────────────────────

function flattenThreadPages(data: { pages: { items: Thread[] }[] } | undefined): Thread[] {
  return data ? data.pages.flatMap((p) => p.items) : [];
}

function flattenMailPages(data: { pages: { items: MailSummary[] }[] } | undefined): MailSummary[] {
  return data ? data.pages.flatMap((p) => p.items) : [];
}

/** Convert MailSummary → minimal Thread shape so ThreadCard can consume it. */
function mailToThread(m: MailSummary): Thread {
  return {
    id: m.id,
    accountId: m.accountId,
    subject: m.subject,
    participants: [m.fromName ?? m.fromEmail],
    mailCount: 1,
    unreadCount: m.isRead ? 0 : 1,
    hasAttachments: m.hasAttachments,
    latestDate: m.dateSent,
    snippet: m.snippet,
    isArchived: false,
    isStarred: false,
  };
}

// ── Virtual row item descriptor ───────────────────────────────────────────────

type RowKind =
  | {
      kind: "thread";
      thread: Thread;
      colorToken: string;
      senderEmail: string;
      senderName: string | null;
    }
  | { kind: "group"; thread: Thread; colorToken: string }
  | { kind: "expanded"; threadId: string; colorToken: string }
  | { kind: "section"; label: string }
  | { kind: "skeleton" }
  | { kind: "load-more" };

// ── Date-section bucketing (prototype groups rows by Today / Yesterday / date) ──

function dayBucket(
  unixSecs: number,
  today: string,
  yesterday: string,
): { key: string; label: string } {
  const d = new Date(unixSecs * 1000);
  const now = new Date();
  const startOf = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diffDays = Math.round((startOf(now) - startOf(d)) / 86_400_000);
  if (diffDays <= 0) return { key: "today", label: today };
  if (diffDays === 1) return { key: "yesterday", label: yesterday };
  const label = d.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: d.getFullYear() === now.getFullYear() ? undefined : "numeric",
  });
  return { key: label, label };
}

// ── Component ─────────────────────────────────────────────────────────────────

export const ThreadList = forwardRef<ThreadListHandle, ThreadListProps>(function ThreadList(
  { accountId, folder, onArchived, onDeleted },
  ref,
) {
  const { t } = useTranslation("list");
  const scrollRef = useRef<HTMLDivElement>(null);

  const density = useUi((s) => s.density);
  const threadFoldingEnabled = useUi((s) => s.threadFoldingEnabled);
  const isThreadExpanded = useUi((s) => s.isThreadExpanded);
  const mailFilter = useUi((s) => s.mailFilter);

  const selectedThreadId = useSelection((s) => s.selectedThreadId);

  // Account lookup → real avatar color/badge per row (prototype color-codes by account).
  const { data: accounts } = useAccounts();
  const acctById = useMemo(() => new Map((accounts ?? []).map((a) => [a.id, a])), [accounts]);
  const todayLabel = t("section_today");
  const yesterdayLabel = t("section_yesterday");

  const compact = density === "compact";
  const ROW_H = compact ? 56 : 72;

  // ── Data queries (mode-dependent) ────────────────────────────────────────

  // Thread fold mode: useThreadsInfinite
  const threadsQuery = useThreadsInfinite(
    threadFoldingEnabled
      ? {
          accountId: accountId ?? undefined,
          folder: folder ?? undefined,
          isArchived: false,
          hasUnread: mailFilter.unreadOnly ? true : undefined,
        }
      : // Disabled when not in fold mode (enabled=false via `skip` pattern).
        // We pass a dummy filter and rely on the condition below.
        { accountId: "__disabled__" },
  );

  // Flat mail mode: useMailsInfinite
  const mailsQuery = useMailsInfinite(
    !threadFoldingEnabled
      ? {
          accountId: accountId ?? undefined,
          folder: folder ?? undefined,
          isUnread: mailFilter.unreadOnly ? true : undefined,
        }
      : { accountId: "__disabled__" },
  );

  const activeQuery = threadFoldingEnabled ? threadsQuery : mailsQuery;
  const { isFetchingNextPage, hasNextPage, fetchNextPage, isFetching, isError } = activeQuery;

  // ── Build virtual row list ────────────────────────────────────────────────

  let rows: RowKind[] = [];
  let lastBucket = "";

  if (threadFoldingEnabled) {
    const threads = flattenThreadPages(
      threadsQuery.data as { pages: { items: Thread[] }[] } | undefined,
    );
    for (const thread of threads) {
      const b = dayBucket(thread.latestDate, todayLabel, yesterdayLabel);
      if (b.key !== lastBucket) {
        rows.push({ kind: "section", label: b.label });
        lastBucket = b.key;
      }
      // colorToken stays account-coded (drives the 3 px stripe); the avatar derives
      // its own per-sender color downstream from each mail's address.
      const colorToken = acctById.get(thread.accountId)?.colorToken ?? "team";
      rows.push({ kind: "group", thread, colorToken });
      if (isThreadExpanded(thread.id)) {
        rows.push({ kind: "expanded", threadId: thread.id, colorToken });
      }
    }
  } else {
    const mails = flattenMailPages(
      mailsQuery.data as { pages: { items: MailSummary[] }[] } | undefined,
    );
    for (const mail of mails) {
      const b = dayBucket(mail.dateSent, todayLabel, yesterdayLabel);
      if (b.key !== lastBucket) {
        rows.push({ kind: "section", label: b.label });
        lastBucket = b.key;
      }
      const colorToken = acctById.get(mail.accountId)?.colorToken ?? "team";
      rows.push({
        kind: "thread",
        thread: mailToThread(mail),
        colorToken,
        senderEmail: mail.fromEmail,
        senderName: mail.fromName,
      });
    }
  }

  if (isFetching && rows.length === 0) {
    rows = Array.from({ length: 8 }, () => ({ kind: "skeleton" as const }));
  } else if (hasNextPage) {
    rows.push({ kind: "load-more" });
  }

  // ── Estimate row height for virtualizer ──────────────────────────────────

  function estimateSize(index: number): number {
    const row = rows[index];
    if (!row) return ROW_H;
    if (row.kind === "section") return 30;
    if (row.kind === "expanded") {
      // We don't know the exact count here; give a generous estimate.
      return ROW_H * 3;
    }
    return ROW_H;
  }

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize,
    overscan: 20,
  });

  // ── Expose scrollToIndex for useMailShortcuts ─────────────────────────────

  useImperativeHandle(
    ref,
    () => ({
      scrollToIndex: (index: number) => {
        virtualizer.scrollToIndex(index, { behavior: "smooth" });
      },
    }),
    [virtualizer],
  );

  // ── Infinite scroll: fetch next page near bottom ──────────────────────────

  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    if (distanceFromBottom < 200 && hasNextPage && !isFetchingNextPage) {
      void fetchNextPage();
    }
  }, [fetchNextPage, hasNextPage, isFetchingNextPage]);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    el.addEventListener("scroll", handleScroll, { passive: true });
    return () => el.removeEventListener("scroll", handleScroll);
  }, [handleScroll]);

  // ── Empty state ───────────────────────────────────────────────────────────

  if (!isFetching && rows.length === 0 && !isError) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 px-8 py-20 text-center">
        <p className="font-display text-2xl italic text-p9">{t("empty_title")}</p>
        <p className="font-body text-sm text-p7">{t("empty_hint")}</p>
      </div>
    );
  }

  // ── Virtual list render ───────────────────────────────────────────────────

  return (
    <div
      ref={scrollRef}
      role="listbox"
      aria-label={t("stream_title")}
      aria-multiselectable="true"
      className="h-full overflow-y-auto"
    >
      <div style={{ height: `${virtualizer.getTotalSize()}px`, position: "relative" }}>
        {virtualizer.getVirtualItems().map((vItem) => {
          const row = rows[vItem.index];
          if (!row) return null;

          return (
            <div
              key={vItem.key}
              data-index={vItem.index}
              ref={virtualizer.measureElement}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                right: 0,
                transform: `translateY(${vItem.start}px)`,
              }}
            >
              {row.kind === "skeleton" && <SkeletonCard compact={compact} />}

              {row.kind === "section" && (
                <div className="flex items-center bg-parchment px-4 pb-1 pt-3">
                  <span className="font-ui text-[9px] font-semibold uppercase tracking-[0.1em] text-p8">
                    {row.label}
                  </span>
                </div>
              )}

              {row.kind === "thread" && (
                <ThreadCard
                  thread={row.thread}
                  colorToken={row.colorToken}
                  senderEmail={row.senderEmail}
                  senderName={row.senderName}
                  isFocused={row.thread.id === selectedThreadId}
                  onArchived={onArchived}
                  onDeleted={onDeleted}
                />
              )}

              {row.kind === "group" && (
                <ThreadGroupCard thread={row.thread} colorToken={row.colorToken} />
              )}

              {row.kind === "expanded" && (
                <ThreadExpanded
                  threadId={row.threadId}
                  colorToken={row.colorToken}
                  onArchived={onArchived}
                  onDeleted={onDeleted}
                />
              )}

              {row.kind === "load-more" && (
                <div className="flex items-center justify-center py-4">
                  {isFetchingNextPage ? (
                    <p className="font-body text-sm text-p7">{t("loading")}</p>
                  ) : (
                    <button
                      type="button"
                      onClick={() => void fetchNextPage()}
                      className="rounded-chip border border-divider bg-surface px-4 py-1.5 font-ui text-xs text-p9 hover:bg-p4"
                    >
                      {t("load_more")}
                    </button>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
});
