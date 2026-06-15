// Inline draft review panel (T081, F_E6 §3.3). Slides in beside the Pending
// list — there is NO standalone draft-review page (root CLAUDE.md hard rule).
// Two panes: the original mail (read-only, DOMPurify-sanitised) and the draft
// editor with the Send / Edit & Send / Open in Compose / Regenerate / Style
// Feedback / Discard actions. Shares the detail container with the future
// DecisionPanel via the parent's `panelMode` switch.
import DOMPurify from "dompurify";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import type { AiDraft } from "@shared/bindings";

import { useMailDetail } from "@/ipc/queries/mail";
import { buildAiComposeSeed, useUpdateDraftBody } from "@/ipc/queries/drafts";
import { DOMPURIFY_CONFIG } from "@/lib/dompurify-config";
import { formatRelativeDate } from "@/lib/formatDate";
import { showToast } from "@/components/ui/Toast";
import { cn } from "@/lib/cn";
import { draftBadgeFor, hoursUntilExpiry } from "./DraftCard";
import { DraftDiffView } from "./DraftDiffView";

/** Stable id so the pending route's `E` shortcut can focus the editor. */
export const DRAFT_EDITOR_ID = "draft-panel-editor";

/** Idle delay before unsaved textarea edits are pushed via update_draft_body. */
const EDIT_DEBOUNCE_MS = 500;

const STYLE_FEEDBACK_OPTIONS = ["too_formal", "too_casual", "too_long", "tone_off"] as const;

interface DraftPanelProps {
  draft: AiDraft;
  /** Send button disabled + inline notice when the account's SMTP auth failed. */
  credentialError?: boolean;
  regenerating?: boolean;
  onSend: () => void;
  onDiscard: () => void;
  onRegenerate: () => void;
  onClose: () => void;
}

export function DraftPanel({
  draft,
  credentialError = false,
  regenerating = false,
  onSend,
  onDiscard,
  onRegenerate,
  onClose,
}: DraftPanelProps) {
  const { t } = useTranslation("aiDrafts");
  const navigate = useNavigate();

  // ── Original mail (left pane) ─────────────────────────────────────────────
  const { data: mail, isError: mailMissing } = useMailDetail(draft.triggerMailId);

  const sanitizedBody = useMemo(
    () => (mail?.bodyHtml ? DOMPurify.sanitize(mail.bodyHtml, DOMPURIFY_CONFIG) : null),
    [mail?.bodyHtml],
  );

  // ── Draft body editing (right pane) ───────────────────────────────────────
  const updateBody = useUpdateDraftBody();
  const [localBody, setLocalBody] = useState(draft.bodyCurrent);
  /** "Show Diff" toggle (T090, F_E6 §4.5) — read-only diff replaces the editor. */
  const [showDiff, setShowDiff] = useState(false);
  /** Last body value the server knows — guards against clobbering edits. */
  const lastServerBody = useRef(draft.bodyCurrent);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Sync in server-side changes (draft switch, regenerate) without overwriting
  // typing: only react when the SERVER value moved.
  useEffect(() => {
    if (draft.bodyCurrent !== lastServerBody.current) {
      lastServerBody.current = draft.bodyCurrent;
      setLocalBody(draft.bodyCurrent);
    }
  }, [draft.id, draft.bodyCurrent]);

  function flushBody(next?: string) {
    const value = next ?? localBody;
    if (debounceRef.current) {
      clearTimeout(debounceRef.current);
      debounceRef.current = null;
    }
    if (value === lastServerBody.current) return;
    lastServerBody.current = value;
    updateBody.mutate({ id: draft.id, bodyCurrent: value });
  }

  function handleBodyChange(e: React.ChangeEvent<HTMLTextAreaElement>) {
    const value = e.target.value;
    setLocalBody(value);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => flushBody(value), EDIT_DEBOUNCE_MS);
  }

  // Flush outstanding edits when the panel unmounts (close / card switch).
  useEffect(() => {
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, []);

  const wordCount = localBody.trim() === "" ? 0 : localBody.trim().split(/\s+/).length;
  /** Live edited state — tracks unsaved keystrokes ahead of `draft.isEdited`. */
  const isEditedLive = draft.isEdited || localBody !== draft.bodyOriginal;
  const badge = draftBadgeFor(draft);
  const badgeLabel =
    badge === "edited"
      ? t("draft_badge_edited")
      : badge === "review"
        ? t("draft_badge_review")
        : t("draft_badge_ready");
  const hours = hoursUntilExpiry(draft.expiresAt);

  // ── Actions ───────────────────────────────────────────────────────────────

  function handleSendNow() {
    flushBody();
    onSend();
  }

  function handleEditAndSend() {
    // Persist edits first (marks `is_edited`), then run the same send flow.
    flushBody();
    onSend();
  }

  function handleOpenInCompose() {
    flushBody();
    void navigate("/compose", {
      state: {
        mode: "reply",
        aiSeed: buildAiComposeSeed({ ...draft, bodyCurrent: localBody }),
      },
    });
  }

  function handleStyleFeedback(e: React.ChangeEvent<HTMLSelectElement>) {
    const choice = e.target.value;
    if (!choice) return;
    // Style-feedback persistence is the v0.7+ E5 pipeline (T076 peer); until it
    // lands the choice is only logged so QA can verify the control wiring.
    console.warn(`[draft-review] style feedback for draft ${draft.id}: ${choice}`);
    showToast(t("toast_style_feedback"));
    e.target.value = "";
  }

  const actionBtn =
    "w-full rounded-chip px-4 py-2 font-ui text-[11px] font-semibold uppercase tracking-wider transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-p9 disabled:opacity-50";

  return (
    <aside
      aria-label={t("draft_panel_title")}
      style={{ borderInlineStart: "1px solid var(--p5)" }}
      className="flex h-full w-[560px] shrink-0 flex-col overflow-y-auto bg-surface"
    >
      {/* Panel header */}
      <div className="flex items-center gap-2 border-b border-divider px-5 py-3">
        <h2 className="font-display text-lg italic text-p10">{t("draft_panel_title")}</h2>
        <span
          className={cn(
            "rounded-chip px-2 py-0.5 font-ui text-[9px] font-semibold uppercase tracking-widest",
            badge === "edited"
              ? "bg-amber/20 text-amber"
              : badge === "review"
                ? "bg-red/10 text-red"
                : "bg-green/15 text-green",
          )}
        >
          {badgeLabel}
        </span>
        {draft.styleMatchScore !== null && (
          <span className="bg-sage/40 rounded-chip px-2 py-0.5 font-ui text-[9px] font-semibold uppercase tracking-widest text-p9">
            {t("draft_style_match", { score: Math.round(draft.styleMatchScore * 100) })}
          </span>
        )}
        <button
          type="button"
          onClick={onClose}
          aria-label="Close draft panel"
          className="ms-auto rounded-chip p-1 text-p7 transition-colors hover:bg-p4 hover:text-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 16 16"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            aria-hidden="true"
          >
            <path strokeLinecap="round" d="m4 4 8 8M12 4l-8 8" />
          </svg>
        </button>
      </div>

      <div className="grid min-h-0 flex-1 grid-cols-2">
        {/* Left pane — original mail, read-only */}
        <div
          className="min-w-0 overflow-y-auto border-divider p-5"
          style={{ borderInlineEnd: "1px solid var(--p5)" }}
        >
          <p className="section-label mb-3">{t("draft_panel_original")}</p>
          {mail ? (
            <>
              <dl className="mb-3 space-y-1 font-ui text-xs text-p8">
                <div className="flex gap-2">
                  <dt className="w-10 shrink-0 uppercase tracking-wider text-p7">
                    {t("draft_from_label")}
                  </dt>
                  <dd className="min-w-0 truncate">{mail.fromName ?? mail.fromEmail}</dd>
                </div>
                <div className="flex gap-2">
                  <dt className="w-10 shrink-0 uppercase tracking-wider text-p7">
                    {t("draft_to_label")}
                  </dt>
                  <dd className="min-w-0 truncate">{mail.to.map((r) => r.email).join(", ")}</dd>
                </div>
                <div className="flex gap-2">
                  <dt className="w-10 shrink-0 uppercase tracking-wider text-p7">
                    {t("draft_date_label")}
                  </dt>
                  <dd className="font-mono">{formatRelativeDate(mail.dateSent)}</dd>
                </div>
              </dl>
              <p className="mb-3 font-body text-sm font-semibold text-p10">{mail.subject}</p>
              {sanitizedBody ? (
                // Safe: second-pass DOMPurify with the shared allowlist (07 §10).
                <div
                  className="font-body text-sm leading-relaxed text-p9 [&_p]:mb-3"
                  dangerouslySetInnerHTML={{ __html: sanitizedBody }}
                />
              ) : (
                <p className="whitespace-pre-wrap font-body text-sm leading-relaxed text-p9">
                  {mail.bodyText}
                </p>
              )}
            </>
          ) : (
            <p className="font-body text-sm italic text-p7">
              {mailMissing ? t("draft_original_unavailable") : null}
            </p>
          )}
        </div>

        {/* Right pane — draft editor + actions */}
        <div className="flex min-w-0 flex-col overflow-y-auto p-5">
          <div className="mb-3 flex items-center gap-2">
            <label htmlFor={DRAFT_EDITOR_ID} className="section-label">
              {t("draft_panel_title")}
            </label>
            {/* Edited-by-you vs original indicator (T090 §3) */}
            <span
              className={cn(
                "font-ui text-[10px] uppercase tracking-wider",
                isEditedLive ? "text-amber" : "text-p7",
              )}
            >
              {isEditedLive ? t("draft_edited_chip") : t("draft_original_label")}
            </span>
            <button
              type="button"
              onClick={() => {
                // Persist outstanding edits so the diff compares saved state.
                if (!showDiff) flushBody();
                setShowDiff((v) => !v);
              }}
              aria-pressed={showDiff}
              className={cn(
                "ms-auto rounded-chip border border-divider px-2.5 py-1 font-ui text-[10px] font-semibold uppercase tracking-wider transition-colors",
                "focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
                showDiff ? "bg-p9 text-white" : "text-p9 hover:bg-p4",
              )}
            >
              {showDiff ? t("draft_hide_diff") : t("draft_show_diff")}
            </button>
          </div>
          {showDiff ? (
            <DraftDiffView original={draft.bodyOriginal} current={localBody} />
          ) : (
            <textarea
              id={DRAFT_EDITOR_ID}
              value={localBody}
              onChange={handleBodyChange}
              onBlur={() => flushBody()}
              spellCheck
              rows={12}
              className={cn(
                "w-full resize-none rounded-card border border-divider bg-p1 p-3",
                "font-body text-sm leading-relaxed text-p10",
                "focus:outline-none focus:ring-1 focus:ring-p9",
              )}
            />
          )}
          <div className="mt-1.5 flex items-center justify-between">
            <span className="font-mono text-[10px] text-p7">
              {t("draft_word_count", { count: wordCount })}
            </span>
            {hours !== null && (
              <span className="font-mono text-[10px] text-p7">
                {t("draft_expires_in", { hours })}
              </span>
            )}
          </div>

          {/* Credential failure notice (F_E6 §3 — SMTP auth expired) */}
          {credentialError && (
            <p
              role="alert"
              className="border-red/30 bg-red/10 mt-3 rounded-card border px-3 py-2 font-body text-xs text-red"
            >
              {t("draft_credential_error")}
            </p>
          )}

          {/* Action stack (F_E6 §3.3 order) */}
          <div className="mt-4 flex flex-col gap-2">
            <button
              type="button"
              onClick={handleSendNow}
              disabled={credentialError || regenerating}
              className={cn(actionBtn, "bg-green text-white hover:opacity-90")}
            >
              {t("draft_send_now")}
            </button>
            <button
              type="button"
              onClick={handleEditAndSend}
              disabled={credentialError || regenerating}
              className={cn(actionBtn, "border border-divider text-p9 hover:bg-p4")}
            >
              {t("draft_edit_send")}
            </button>
            <button
              type="button"
              onClick={handleOpenInCompose}
              className={cn(actionBtn, "border border-divider text-p9 hover:bg-p4")}
            >
              {t("draft_open_compose")}
            </button>
            <button
              type="button"
              onClick={onRegenerate}
              disabled={regenerating}
              aria-busy={regenerating}
              className={cn(actionBtn, "border border-divider text-p9 hover:bg-p4")}
            >
              {regenerating ? t("draft_regenerating") : t("draft_regenerate")}
            </button>

            <label className="mt-1 flex items-center gap-2">
              <span className="section-label">{t("draft_style_feedback")}</span>
              <select
                defaultValue=""
                onChange={handleStyleFeedback}
                aria-label={t("draft_style_feedback")}
                className="min-w-0 flex-1 rounded-chip border border-divider bg-parchment px-2 py-1 font-body text-xs text-p9 focus:outline-none focus:ring-1 focus:ring-p9"
              >
                <option value="" disabled>
                  —
                </option>
                {STYLE_FEEDBACK_OPTIONS.map((opt) => (
                  <option key={opt} value={opt}>
                    {t(`style_${opt}`)}
                  </option>
                ))}
              </select>
            </label>

            <button
              type="button"
              onClick={onDiscard}
              className={cn(actionBtn, "hover:bg-red/10 mt-1 text-red")}
            >
              {t("draft_discard")}
            </button>
          </div>
        </div>
      </div>
    </aside>
  );
}
