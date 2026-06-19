// Compose page top bar (T044, F_G4 §4.1–§4.2). Shows the From-account selector,
// a mode badge, the Cc/Bcc toggle, and a close/discard button. The save-draft
// button is intentionally omitted here — draft saving is automatic (T045); a
// manual trigger lives in ComposeFooter. When the buffer was seeded from an AI
// draft (T078, E1), a Regenerate button re-runs generation in place.

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useAccounts } from "@/ipc/queries/accounts";
import { useRegenerateDraft } from "@/ipc/queries/drafts";
import { useCompose } from "@/stores/compose";
import { markdownToPlainText } from "@/lib/markdown";
import { plainTextToHtml } from "@/lib/richText";
import { cn } from "@/lib/cn";

/** After this many regenerations, suggest writing manually (F_E1 §4.6). */
const REGEN_HINT_THRESHOLD = 3;

// ── Types ────────────────────────────────────────────────────────────────────

export type ComposeMode = "new" | "reply" | "reply-all" | "forward";

interface ComposeToolbarProps {
  mode: ComposeMode;
  /** Called when the user clicks the × close button. */
  onClose: () => void;
}

// ── Mode badge ───────────────────────────────────────────────────────────────

const MODE_LABELS: Record<ComposeMode, string> = {
  new: "New",
  reply: "Reply",
  "reply-all": "Reply All",
  forward: "Forward",
};

const MODE_COLORS: Record<ComposeMode, string> = {
  new: "bg-p9 text-white",
  reply: "bg-slate text-white",
  "reply-all": "bg-slate text-white",
  forward: "bg-amber/20 text-p9",
};

// ── Component ────────────────────────────────────────────────────────────────

export function ComposeToolbar({ mode, onClose }: ComposeToolbarProps) {
  const { t } = useTranslation("compose");
  const { t: tAi } = useTranslation("aiDrafts");

  const accountId = useCompose((s) => s.accountId);
  const update = useCompose((s) => s.update);
  const ccVisible = useCompose((s) => s.ccVisible);
  const setCcVisible = useCompose((s) => s.setCcVisible);
  const aiDraftId = useCompose((s) => s.aiDraftId);
  const setAiRegenerating = useCompose((s) => s.setAiRegenerating);

  const { data: accounts = [] } = useAccounts();
  const activeAccounts = accounts.filter((a) => a.isActive);

  // ── E1 regenerate (T078, F_E1 §4.6) ─────────────────────────────────────
  const regenerate = useRegenerateDraft();
  /** Session-scoped retry counter — useRef per CLAUDE.md (no localStorage). */
  const regenCountRef = useRef(0);
  const [showRegenHint, setShowRegenHint] = useState(false);

  // Mirror the in-flight flag into the store so ComposeFooter disables Send.
  useEffect(() => {
    setAiRegenerating(regenerate.isPending);
  }, [regenerate.isPending, setAiRegenerating]);

  function handleRegenerate() {
    if (!aiDraftId || regenerate.isPending) return;
    regenerate.mutate(
      { id: aiDraftId },
      {
        onSuccess: (draft) => {
          const text = markdownToPlainText(draft.bodyCurrent);
          update({ body: text, bodyHtml: plainTextToHtml(text), aiDraftId: draft.id });
          regenCountRef.current += 1;
          if (regenCountRef.current >= REGEN_HINT_THRESHOLD) setShowRegenHint(true);
        },
      },
    );
  }

  function handleFromChange(e: React.ChangeEvent<HTMLSelectElement>) {
    update({ accountId: e.target.value || null });
  }

  return (
    <div className="flex items-center gap-3 border-b border-divider bg-surface px-5 py-3">
      {/* From selector */}
      <label className="section-label shrink-0">{t("from_label")}</label>
      <select
        value={accountId ?? ""}
        onChange={handleFromChange}
        aria-label={t("from_label")}
        className={cn(
          "min-w-0 flex-1 rounded-chip border border-divider bg-parchment px-2.5 py-1",
          "font-body text-sm text-p10 focus:outline-none focus:ring-1 focus:ring-p9",
        )}
      >
        {!accountId && (
          <option value="" disabled>
            Select account…
          </option>
        )}
        {activeAccounts.map((acct) => (
          <option key={acct.id} value={acct.id}>
            {acct.displayName} &lt;{acct.email}&gt;
          </option>
        ))}
      </select>

      {/* Mode badge */}
      <span
        className={cn(
          "shrink-0 rounded-chip px-2 py-0.5 font-ui text-[9px] font-semibold uppercase tracking-wider",
          MODE_COLORS[mode],
        )}
      >
        {MODE_LABELS[mode]}
      </span>

      {/* Regenerate (only when seeded from an AI draft, T078) */}
      {aiDraftId && (
        <div className="flex shrink-0 items-center gap-2">
          <button
            type="button"
            onClick={handleRegenerate}
            disabled={regenerate.isPending}
            aria-busy={regenerate.isPending}
            aria-label={tAi("e1_regenerate")}
            className={cn(
              "flex items-center gap-1.5 rounded-chip px-2.5 py-1 font-ui text-[10px] uppercase tracking-wider transition-colors",
              "text-p9 hover:bg-p4 hover:text-p10 disabled:opacity-60",
              "focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
            )}
          >
            {regenerate.isPending && (
              <svg
                width="12"
                height="12"
                viewBox="0 0 16 16"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.5"
                className="animate-spin"
                aria-hidden="true"
              >
                <path strokeLinecap="round" d="M8 1.5A6.5 6.5 0 1 1 1.5 8" />
              </svg>
            )}
            {tAi("e1_regenerate")}
          </button>
          {showRegenHint && (
            <span className="font-body text-xs italic text-p7">{tAi("e1_regen_hint")}</span>
          )}
        </div>
      )}

      {/* Cc/Bcc toggle */}
      <button
        type="button"
        onClick={() => setCcVisible(!ccVisible)}
        aria-pressed={ccVisible}
        className={cn(
          "shrink-0 rounded-chip px-2.5 py-1 font-ui text-[10px] uppercase tracking-wider transition-colors",
          ccVisible ? "bg-p4 text-p10" : "text-p7 hover:bg-p4 hover:text-p10",
        )}
      >
        {t("add_cc")}
      </button>

      {/* Close */}
      <button
        type="button"
        onClick={onClose}
        aria-label="Discard and close"
        className="ms-1 rounded-chip p-1 text-p7 transition-colors hover:bg-p4 hover:text-p10"
      >
        <svg
          xmlns="http://www.w3.org/2000/svg"
          viewBox="0 0 20 20"
          fill="currentColor"
          className="h-4 w-4"
          aria-hidden
        >
          <path d="M6.28 5.22a.75.75 0 00-1.06 1.06L8.94 10l-3.72 3.72a.75.75 0 101.06 1.06L10 11.06l3.72 3.72a.75.75 0 101.06-1.06L11.06 10l3.72-3.72a.75.75 0 00-1.06-1.06L10 8.94 6.28 5.22z" />
        </svg>
      </button>
    </div>
  );
}
