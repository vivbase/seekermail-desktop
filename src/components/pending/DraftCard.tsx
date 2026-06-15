// "Draft Ready" card on the Pending page (T081, F_E6 §3.2, root CLAUDE.md
// "Pending Page — Two Card Types"). Rendered alongside the "Needs Decision"
// cards; `data-type="draft"` is the E2E selector contract. The green inline-
// start border distinguishes draft cards from decision cards.
import { useTranslation } from "react-i18next";
import type { Account, AiDraft } from "@shared/bindings";

import { accountColorClass, type AccountColorToken } from "@/lib/accountColor";
import { cn } from "@/lib/cn";

/** Badge variants per F_E6 §3.2. */
export type DraftBadge = "ready" | "edited" | "review";

/** Drafts expiring within this window get the "Review Needed" badge. */
const REVIEW_NEEDED_WINDOW_SECS = 12 * 3600;

/**
 * Map a draft to its badge: user-edited drafts show EDITED, drafts close to
 * expiry escalate to REVIEW NEEDED, everything else is DRAFT READY.
 */
export function draftBadgeFor(draft: AiDraft, nowUnix = Math.floor(Date.now() / 1000)): DraftBadge {
  if (draft.isEdited || draft.status === "edited") return "edited";
  if (draft.expiresAt !== null && draft.expiresAt - nowUnix <= REVIEW_NEEDED_WINDOW_SECS) {
    return "review";
  }
  return "ready";
}

/** Whole hours until a draft expires (floored at 0 for display). */
export function hoursUntilExpiry(
  expiresAt: number | null,
  nowUnix = Math.floor(Date.now() / 1000),
): number | null {
  if (expiresAt === null) return null;
  return Math.max(0, Math.ceil((expiresAt - nowUnix) / 3600));
}

const BADGE_CLASS: Record<DraftBadge, string> = {
  ready: "bg-green/15 text-green",
  edited: "bg-amber/20 text-amber",
  review: "bg-red/10 text-red",
};

const PREVIEW_CHARS = 140;

interface DraftCardProps {
  draft: AiDraft;
  /** Owning account, for the color chip + display name. */
  account?: Account;
  selected?: boolean;
  onOpen: (draftId: string) => void;
}

export function DraftCard({ draft, account, selected = false, onOpen }: DraftCardProps) {
  const { t } = useTranslation("aiDrafts");

  const badge = draftBadgeFor(draft);
  const badgeLabel =
    badge === "edited"
      ? t("draft_badge_edited")
      : badge === "review"
        ? t("draft_badge_review")
        : t("draft_badge_ready");

  const hours = hoursUntilExpiry(draft.expiresAt);
  const colorToken = (account?.colorToken as AccountColorToken | undefined) ?? "slate";
  const preview = draft.bodyCurrent.slice(0, PREVIEW_CHARS);

  function handleKeyDown(e: React.KeyboardEvent<HTMLDivElement>) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      onOpen(draft.id);
    }
  }

  return (
    <div
      data-type="draft"
      role="button"
      tabIndex={0}
      aria-label={`${draft.subject} — ${badgeLabel}`}
      aria-pressed={selected}
      onClick={() => onOpen(draft.id)}
      onKeyDown={handleKeyDown}
      style={{ borderInlineStart: "3px solid var(--green)" }}
      className={cn(
        "cursor-pointer rounded-card border border-divider bg-surface p-5 shadow-card transition-colors",
        "hover:border-p7 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
        selected && "border-p9",
      )}
    >
      {/* Header: account chip + counterpart + badge */}
      <div className="flex items-center gap-2.5">
        <span
          aria-hidden="true"
          className={cn(
            "flex h-6 w-6 shrink-0 items-center justify-center rounded-avatar font-ui text-[10px] font-semibold",
            accountColorClass(colorToken),
          )}
        >
          {account?.badgeLabel ?? "W"}
        </span>
        <span className="min-w-0 truncate font-ui text-xs text-p8">
          {account?.displayName ?? draft.accountId}
          <span className="text-p7"> · {draft.toAddr.email}</span>
        </span>
        <span
          className={cn(
            "ms-auto shrink-0 rounded-chip px-2 py-0.5 font-ui text-[9px] font-semibold uppercase tracking-widest",
            BADGE_CLASS[badge],
          )}
        >
          {badgeLabel}
        </span>
      </div>

      {/* Subject */}
      <p className="mt-3 truncate font-body text-sm font-semibold text-p10">{draft.subject}</p>

      {/* Draft body preview — green-indented per F_E6 §3.2 */}
      <p
        style={{ borderInlineStart: "2px solid var(--green)", paddingInlineStart: "10px" }}
        className="mt-2 line-clamp-2 font-body text-xs leading-relaxed text-p8"
      >
        {preview}
      </p>

      {/* Footer: expiry countdown + review CTA */}
      <div className="mt-3 flex items-center justify-between gap-2">
        {hours !== null ? (
          <span className="font-mono text-[10px] text-p7">{t("draft_expires_in", { hours })}</span>
        ) : (
          <span />
        )}
        <span className="font-ui text-[10px] font-semibold uppercase tracking-wider text-green">
          {t("draft_review_cta")}
        </span>
      </div>
    </div>
  );
}
