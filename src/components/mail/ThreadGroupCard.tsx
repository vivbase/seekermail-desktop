// Foldable thread group header (T040, F_G2 §4.5).
// Represents multiple mails that share a thread_id in fold mode.
// Fixed height 72 px (comfortable) / 56 px (compact).
import { useCallback } from "react";
import { useTranslation } from "react-i18next";

import type { Thread } from "@shared/bindings";
import { cn } from "@/lib/cn";
import { accountColorClass, type AccountColorToken } from "@/lib/accountColor";
import { formatMailDate } from "@/lib/formatDate";
import { useUi } from "@/stores/ui";

function colorStripeClass(token: string): string {
  const map: Record<string, string> = {
    terra: "bg-terra",
    slate: "bg-slate",
    sage: "bg-sage",
    amber: "bg-amber",
    team: "bg-p9",
  };
  return map[token] ?? "bg-p7";
}

function ChevronIcon({ expanded }: { expanded: boolean }) {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className={cn("shrink-0 transition-transform", expanded && "rotate-90")}
    >
      <polyline points="9 18 15 12 9 6" />
    </svg>
  );
}

export interface ThreadGroupCardProps {
  thread: Thread;
  colorToken: string;
  badgeLabel: string;
}

export function ThreadGroupCard({ thread, colorToken }: ThreadGroupCardProps) {
  const { t } = useTranslation("list");
  const density = useUi((s) => s.density);
  const compact = density === "compact";
  const isExpanded = useUi((s) => s.isThreadExpanded(thread.id));
  const toggleExpandedThread = useUi((s) => s.toggleExpandedThread);

  const handleToggle = useCallback(() => {
    toggleExpandedThread(thread.id);
  }, [thread.id, toggleExpandedThread]);

  const unread = thread.unreadCount > 0;

  // Up to 3 participant initials as avatar chips
  const avatarChips = thread.participants.slice(0, 3).map((p, i) => {
    const initial = p.trim().charAt(0).toUpperCase();
    return (
      <span
        key={i}
        aria-hidden="true"
        className={cn(
          "flex h-7 w-7 shrink-0 items-center justify-center rounded-avatar text-[10px] font-semibold",
          accountColorClass((colorToken as AccountColorToken) ?? "team"),
        )}
      >
        {initial}
      </span>
    );
  });

  const moreCount = thread.participants.length - 3;

  return (
    <button
      type="button"
      aria-expanded={isExpanded}
      aria-label={`${thread.subject}, ${thread.mailCount} messages`}
      onClick={handleToggle}
      className={cn(
        "thread-meta group relative flex w-full cursor-pointer items-center gap-3 border-b border-divider bg-surface px-4 transition-colors",
        "hover:bg-p2 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
        compact ? "h-14" : "h-[72px]",
        unread && "font-medium",
      )}
    >
      {/* Account color stripe */}
      <div
        className={cn("absolute inset-y-0 start-0 w-[3px]", colorStripeClass(colorToken))}
        aria-hidden="true"
      />

      {/* Expand/collapse chevron + avatar chips */}
      <div className="ms-3 flex shrink-0 items-center gap-1">
        <ChevronIcon expanded={isExpanded} />
        <div className="flex items-center -space-x-1.5">{avatarChips}</div>
        {moreCount > 0 && <span className="font-mono text-[10px] text-p7">+{moreCount}</span>}
      </div>

      {/* Main content */}
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <div className="flex items-baseline justify-between gap-2">
          <span
            className={cn(
              "truncate font-ui text-sm",
              unread ? "font-semibold text-p10" : "text-p9",
            )}
          >
            {thread.subject}
          </span>
          <div className="flex shrink-0 items-center gap-1.5">
            {/* Mail count badge */}
            <span className="inline-flex h-4 min-w-[1rem] items-center justify-center rounded-avatar bg-p5 px-1 font-mono text-[10px] text-p9">
              {thread.mailCount}
            </span>
            <time
              dateTime={new Date(thread.latestDate * 1000).toISOString()}
              className="font-mono text-xs text-p7"
            >
              {formatMailDate(thread.latestDate)}
            </time>
          </div>
        </div>

        {/* Snippet / more-in-thread label */}
        <p className="truncate font-body text-xs text-p7">
          {thread.snippet ? thread.snippet.slice(0, 120) : ""}
        </p>

        <p className="font-ui text-[10px] uppercase tracking-[.08em] text-p8">
          {t("more_in_thread", { count: thread.mailCount })}
        </p>
      </div>
    </button>
  );
}
