// L0 mail card — one Thread row in the virtualized stream (T037, F_G1 §4.1).
// Fixed height: 72 px (comfortable) / 56 px (compact). Uses design tokens only.
// Mutations are wired via hooks — no direct ipc() calls.
import { useCallback } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";

import type { Thread } from "@shared/bindings";
import { cn } from "@/lib/cn";
import { formatMailDate } from "@/lib/formatDate";
import { SenderAvatar } from "./SenderAvatar";
import { useUi } from "@/stores/ui";
import { useSelection } from "@/stores/selection";
import {
  useSetMailRead,
  useSetMailStarred,
  useArchiveMail,
  useDeleteMail,
} from "@/ipc/queries/mail";
import { useAiDraftTriggerMailIds } from "@/ipc/queries/drafts";

// ── Account color stripe helper ───────────────────────────────────────────────

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

// ── Icons (inline SVG, no external CDN) ─────────────────────────────────────

function StarIcon({ filled }: { filled: boolean }) {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill={filled ? "currentColor" : "none"}
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
    </svg>
  );
}

function PaperclipIcon() {
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
    >
      <path d="M21.44 11.05l-9.19 9.19a6 6 0 01-8.49-8.49l9.19-9.19a4 4 0 015.66 5.66l-9.2 9.19a2 2 0 01-2.83-2.83l8.49-8.48" />
    </svg>
  );
}

function ArchiveIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <polyline points="21 8 21 21 3 21 3 8" />
      <rect x="1" y="3" width="22" height="5" />
      <line x1="10" y1="12" x2="14" y2="12" />
    </svg>
  );
}

function TrashIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <polyline points="3 6 5 6 21 6" />
      <path d="M19 6l-1 14H6L5 6" />
      <path d="M10 11v6M14 11v6" />
      <path d="M9 6V4h6v2" />
    </svg>
  );
}

function MailReadIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M4 4h16c1.1 0 2 .9 2 2v12c0 1.1-.9 2-2 2H4c-1.1 0-2-.9-2-2V6c0-1.1.9-2 2-2z" />
      <polyline points="22 6 12 13 2 6" />
    </svg>
  );
}

// ── Component ────────────────────────────────────────────────────────────────

export interface ThreadCardProps {
  thread: Thread;
  /** Account colorToken string for the 3 px color stripe (stays account-coded). */
  colorToken: string;
  /** Sender email — drives the avatar initial + its deterministic panel color. */
  senderEmail: string;
  /** Sender display name — used for the initial only when the email is blank. */
  senderName?: string | null;
  /** Optional fetched avatar image URL (none wired yet — falls back to initial). */
  avatarUrl?: string | null;
  /** Whether this card is currently keyboard-focused in the list. */
  isFocused?: boolean;
  /** Called when archive completes — parent can show UndoToast. */
  onArchived?: (threadId: string) => void;
  /** Called when delete completes — parent can show UndoToast. */
  onDeleted?: (threadId: string) => void;
}

export function ThreadCard({
  thread,
  colorToken,
  senderEmail,
  senderName,
  avatarUrl,
  isFocused = false,
  onArchived,
  onDeleted,
}: ThreadCardProps) {
  const { t } = useTranslation(["list", "aiDrafts"]);
  const navigate = useNavigate();

  // T083 L0 badge stub: mark threads whose representative mail has a pending
  // AI draft. Set lookup is O(1) per row (memoised in the hook).
  const aiDraftMailIds = useAiDraftTriggerMailIds();
  const hasAiDraft = aiDraftMailIds.has(thread.id);

  const density = useUi((s) => s.density);
  const compact = density === "compact";

  const selectedThreadId = useSelection((s) => s.selectedThreadId);
  const selectThread = useSelection((s) => s.selectThread);
  const toggleChecked = useSelection((s) => s.toggleChecked);
  const isChecked = useSelection((s) => s.isChecked);
  const checked = isChecked(thread.id);

  const setMailRead = useSetMailRead();
  const setMailStarred = useSetMailStarred();
  const archiveMail = useArchiveMail();
  const deleteMail = useDeleteMail();

  const isSelected = selectedThreadId === thread.id;
  const unread = thread.unreadCount > 0;

  // Derive the representative mail id from the thread for mutations.
  // Thread.id is used as a stand-in until we have a representative mailId field.
  const repMailId = thread.id;

  const handleClick = useCallback(
    (e: React.MouseEvent) => {
      if ((e.target as HTMLElement).closest("[data-action]")) return;
      if (e.shiftKey || e.metaKey || e.ctrlKey) {
        toggleChecked(thread.id);
        return;
      }
      selectThread(thread.id);
      navigate(`/mail/${thread.id}`);
    },
    [navigate, selectThread, thread.id, toggleChecked],
  );

  const handleCheckboxChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      e.stopPropagation();
      toggleChecked(thread.id);
    },
    [thread.id, toggleChecked],
  );

  const handleStar = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      void setMailStarred.mutate({ mailId: repMailId, isStarred: !thread.isStarred });
    },
    [repMailId, setMailStarred, thread.isStarred],
  );

  const handleMarkRead = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      void setMailRead.mutate({ mailId: repMailId, isRead: !unread });
    },
    [repMailId, setMailRead, unread],
  );

  const handleArchive = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      void archiveMail.mutate(repMailId, {
        onSuccess: () => onArchived?.(thread.id),
      });
    },
    [archiveMail, onArchived, repMailId, thread.id],
  );

  const handleDelete = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      void deleteMail.mutate(repMailId, {
        onSuccess: () => onDeleted?.(thread.id),
      });
    },
    [deleteMail, onDeleted, repMailId, thread.id],
  );

  // Sender display: use participants[0] or fall back to thread subject words.
  const senderDisplay = thread.participants[0] ?? thread.subject.split(" ").slice(0, 2).join(" ");

  // Snippet capped at 120 characters.
  const snippet =
    thread.snippet && thread.snippet.length > 120
      ? thread.snippet.slice(0, 120) + "…"
      : (thread.snippet ?? "");

  const ariaLabel = [unread ? "Unread:" : "", thread.subject, "from", senderDisplay]
    .filter(Boolean)
    .join(" ");

  return (
    <div
      role="option"
      aria-selected={isSelected || checked}
      aria-keyshortcuts="e # s u"
      aria-label={ariaLabel}
      tabIndex={isFocused ? 0 : -1}
      onClick={handleClick}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          selectThread(thread.id);
          navigate(`/mail/${thread.id}`);
        }
      }}
      className={cn(
        "thread-meta group relative flex w-full cursor-pointer items-center gap-3 border-b border-divider bg-surface px-4 transition-colors",
        "hover:bg-p2 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
        compact ? "h-14" : "h-[72px]",
        (isSelected || checked) && "bg-p2",
        unread && "font-medium",
      )}
    >
      {/* Account color stripe (3 px inline-start) */}
      <div
        className={cn("absolute inset-y-0 start-0 w-[3px]", colorStripeClass(colorToken))}
        aria-hidden="true"
      />

      {/* Checkbox (appears on hover or when checked) */}
      <label
        className={cn(
          "relative ms-3 flex shrink-0 cursor-pointer items-center",
          !checked && "opacity-0 group-hover:opacity-100",
        )}
        onClick={(e) => e.stopPropagation()}
      >
        <span className="sr-only">
          {checked ? t("bulk_clear") : t("selected_count", { count: 0 })}
        </span>
        <input
          type="checkbox"
          checked={checked}
          onChange={handleCheckboxChange}
          className="h-4 w-4 cursor-pointer rounded border-divider accent-p9"
          data-action="checkbox"
        />
      </label>

      {/* Avatar — sender initial + per-sender panel color (not the account badge) */}
      <SenderAvatar
        email={senderEmail}
        name={senderName}
        avatarUrl={avatarUrl}
        className="text-xs"
      />

      {/* Main content */}
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        {/* Row 1: sender + date + unread count */}
        <div className="flex items-baseline justify-between gap-2">
          <span
            className={cn(
              "truncate font-ui text-sm",
              unread ? "font-semibold text-p10" : "text-p9",
            )}
          >
            {senderDisplay}
          </span>
          <div className="flex shrink-0 items-center gap-1.5">
            {hasAiDraft && (
              <span
                data-testid="ai-draft-chip"
                className="rounded-chip bg-green px-1.5 py-0.5 font-ui text-[8px] font-semibold uppercase tracking-widest text-white"
              >
                {t("aiDrafts:thread_ai_draft_chip")}
              </span>
            )}
            {unread && thread.unreadCount > 1 && (
              <span className="inline-flex h-4 min-w-[1rem] items-center justify-center rounded-avatar bg-p9 px-1 font-mono text-[10px] text-surface">
                {thread.unreadCount}
              </span>
            )}
            <time
              dateTime={new Date(thread.latestDate * 1000).toISOString()}
              className={cn("font-mono text-xs", unread ? "font-semibold text-p9" : "text-p7")}
            >
              {formatMailDate(thread.latestDate)}
            </time>
          </div>
        </div>

        {/* Row 2: subject */}
        <p className={cn("truncate text-sm", unread ? "text-p10" : "text-p9")}>{thread.subject}</p>

        {/* Row 3: snippet + status icons */}
        <div className="flex items-center gap-1.5">
          <p className="min-w-0 flex-1 truncate font-body text-xs text-p7">{snippet}</p>
          <div className="flex shrink-0 items-center gap-1 text-p7">
            {thread.hasAttachments && (
              <span aria-label={t("attachments_label")}>
                <PaperclipIcon />
              </span>
            )}
            {unread && <span aria-hidden="true" className="h-2 w-2 rounded-avatar bg-p9" />}
          </div>
        </div>
      </div>

      {/* Hover quick-action buttons (appear on group-hover) */}
      <div
        className="absolute end-3 top-1/2 flex -translate-y-1/2 items-center gap-1 opacity-0 transition-opacity group-hover:opacity-100"
        aria-label="Quick actions"
        data-action="quick-actions"
      >
        <button
          type="button"
          data-action="mark-read"
          aria-label={unread ? t("bulk_read") : t("bulk_unread")}
          onClick={handleMarkRead}
          className="rounded-chip p-1.5 text-p7 transition-colors hover:bg-p4 hover:text-p9"
        >
          <MailReadIcon />
        </button>
        <button
          type="button"
          data-action="star"
          aria-label="Star"
          aria-pressed={thread.isStarred}
          onClick={handleStar}
          className={cn(
            "rounded-chip p-1.5 transition-colors hover:bg-p4",
            thread.isStarred ? "text-amber" : "text-p7 hover:text-p9",
          )}
        >
          <StarIcon filled={thread.isStarred} />
        </button>
        <button
          type="button"
          data-action="archive"
          aria-label={t("bulk_archive")}
          onClick={handleArchive}
          className="rounded-chip p-1.5 text-p7 transition-colors hover:bg-p4 hover:text-p9"
        >
          <ArchiveIcon />
        </button>
        <button
          type="button"
          data-action="delete"
          aria-label={t("bulk_delete")}
          onClick={handleDelete}
          className="rounded-chip p-1.5 text-p7 transition-colors hover:bg-p4 hover:text-red"
        >
          <TrashIcon />
        </button>
      </div>
    </div>
  );
}
