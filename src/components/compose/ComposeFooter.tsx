// Compose action bar (T044, F_G4 §§4.8–4.10). Houses Send, Discard, the
// pre-send warning strip, and the undo-send banner (10 s countdown). Also
// exposes a "Save Draft" manual trigger that calls the autosave's saveNow().

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

import type { SendMailParams } from "@shared/bindings";
import { useCompose } from "@/stores/compose";
import { useDeleteDraft } from "@/ipc/queries/drafts";
import ConfirmDialog from "@/components/ui/ConfirmDialog";
import { DraftSaveBadge } from "./DraftSaveBadge";
import { AiDraftButton } from "./AiDraftButton";
import type { ComposeMode } from "./ComposeToolbar";

import type { UseSendMailReturn } from "@/hooks/useSendMail";
import type { UseDraftAutosaveReturn } from "@/hooks/useDraftAutosave";
import { cn } from "@/lib/cn";

// ── Shared warning type ──────────────────────────────────────────────────────

export interface ValidationWarning {
  message: string;
}

// ── Types ────────────────────────────────────────────────────────────────────

export interface ComposeFooterProps {
  sender: UseSendMailReturn;
  autosave: UseDraftAutosaveReturn;
  /** Compose mode — gates the AI Draft affordance (hidden for reply, D3). */
  mode: ComposeMode;
  /** Blocking validation errors to display (prevent send). */
  blockingErrors: string[];
  /** Soft warnings the user can acknowledge before sending. */
  warnings: ValidationWarning[];
  /**
   * Build the SendMailParams from the current editor state.
   * Used when the user clicks "Send Anyway" to bypass warnings.
   */
  buildParams: () => SendMailParams;
  /** Clear outstanding warning state so send proceeds. */
  onClearWarnings: () => void;
  /**
   * Called when the Send button is clicked. The parent handles validation
   * and warning logic; if warnings are present the parent surfaces them and
   * this callback is NOT called again — "Send Anyway" calls onForcesSend
   * instead.
   */
  onSendClick: () => void;
  /**
   * When true the toolbar close button requested a discard — ComposeFooter
   * opens its confirmation dialog immediately and calls onDiscardRequestHandled
   * when done.
   */
  discardRequested?: boolean;
  onDiscardRequestHandled?: () => void;
}

// ── Countdown display ────────────────────────────────────────────────────────

function formatCountdown(ms: number): string {
  const seconds = Math.ceil(ms / 1000);
  return `${seconds}s`;
}

// ── Component ────────────────────────────────────────────────────────────────

export function ComposeFooter({
  sender,
  autosave,
  mode,
  blockingErrors,
  warnings,
  buildParams,
  onClearWarnings,
  onSendClick,
  discardRequested = false,
  onDiscardRequestHandled,
}: ComposeFooterProps) {
  const { t } = useTranslation("compose");
  const navigate = useNavigate();

  const draftId = useCompose((s) => s.draftId);
  const aiRegenerating = useCompose((s) => s.aiRegenerating);
  const reset = useCompose((s) => s.reset);

  const { mutate: deleteDraft } = useDeleteDraft();

  const [discardDialogOpen, setDiscardDialogOpen] = useState(false);

  // Open the discard dialog when the toolbar close button signals a request.
  useEffect(() => {
    if (discardRequested) {
      setDiscardDialogOpen(true);
      onDiscardRequestHandled?.();
    }
  }, [discardRequested, onDiscardRequestHandled]);

  const isSending = sender.status.phase === "sending";
  const isUndoPhase = sender.status.phase === "undo";

  // ── Send handler ───────────────────────────────────────────────────────

  async function handleSend() {
    if (blockingErrors.length > 0) return; // Guard: toolbar should have shown errors.
    try {
      const params = buildParams();
      await sender.send(params);
      // On success: delete the draft from the backend and reset the store.
      if (draftId) deleteDraft(draftId);
      reset();
    } catch {
      // Error state is managed by useSendMail; nothing to do here.
    }
  }

  // ── Discard handler ────────────────────────────────────────────────────

  function handleDiscardConfirm() {
    setDiscardDialogOpen(false);
    if (draftId) deleteDraft(draftId);
    reset();
    // Fixed parent: compose returns to Inbox (root CLAUDE.md back-button rule).
    void navigate("/all-mail");
  }

  // ── Manual save now ────────────────────────────────────────────────────

  async function handleSaveNow() {
    await autosave.saveNow();
  }

  return (
    <>
      {/* Soft warning strip (Stage 2 validation) */}
      {warnings.length > 0 && (
        <div className="border-amber/30 bg-amber/10 border-t px-5 py-2.5">
          <div className="flex flex-wrap items-start gap-4">
            <div className="flex-1">
              {warnings.map((w, i) => (
                <p key={i} className="font-body text-xs text-p9">
                  {w.message}
                </p>
              ))}
            </div>
            <div className="flex shrink-0 gap-2">
              <button
                type="button"
                onClick={onClearWarnings}
                className="rounded-chip px-3 py-1 font-ui text-[10px] uppercase tracking-wider text-p8 hover:bg-p4"
              >
                Go back
              </button>
              <button
                type="button"
                onClick={async () => {
                  onClearWarnings();
                  await handleSend();
                }}
                className="rounded-chip bg-p9 px-3 py-1 font-ui text-[10px] uppercase tracking-wider text-white hover:bg-p10"
              >
                Send Anyway
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Undo-send banner (10 s countdown after successful send) */}
      {isUndoPhase && (
        <div
          role="status"
          aria-live="polite"
          className="border-green/30 bg-green/10 flex items-center justify-between border-t px-5 py-2.5"
        >
          <p className="font-body text-sm text-p9">
            {t("sent")}{" "}
            {sender.status.phase === "undo" && (
              <span className="font-mono text-xs text-p7">
                ({formatCountdown(sender.status.remainingMs)})
              </span>
            )}
          </p>
          <button
            type="button"
            onClick={sender.cancel}
            className="rounded-chip border border-p5 bg-surface px-3 py-1 font-ui text-[10px] uppercase tracking-wider text-p9 hover:bg-p4"
          >
            {t("cancel_send")}
          </button>
        </div>
      )}

      {/* Action row */}
      <div className="flex items-center gap-2 border-t border-divider px-5 py-3">
        {/* Draft save badge */}
        <DraftSaveBadge status={autosave.status} />

        <div className="flex-1" />

        {/* Manual save draft */}
        <button
          type="button"
          onClick={() => void handleSaveNow()}
          disabled={isSending}
          className="rounded-chip px-3 py-1.5 font-ui text-[10px] uppercase tracking-wider text-p7 hover:bg-p4 hover:text-p10 disabled:opacity-40"
        >
          Save Draft
        </button>

        {/* Discard */}
        <button
          type="button"
          onClick={() => setDiscardDialogOpen(true)}
          disabled={isSending}
          className="rounded-chip px-3 py-1.5 font-ui text-[10px] uppercase tracking-wider text-p7 hover:bg-p4 hover:text-red disabled:opacity-40"
        >
          {t("discard")}
        </button>

        {/* AI Draft — intent-capture generation (analysis/57 §7) */}
        <AiDraftButton mode={mode} />

        {/* AI Polish (disabled placeholder, v0.5+) */}
        <button
          type="button"
          disabled
          aria-disabled="true"
          title="AI Polish — available in v0.5"
          className="cursor-not-allowed rounded-chip px-3 py-1.5 font-ui text-[10px] uppercase tracking-wider text-p7 opacity-40"
        >
          AI Polish
        </button>

        {/* Send */}
        <button
          type="button"
          onClick={onSendClick}
          disabled={isSending || aiRegenerating}
          className={cn(
            "rounded-chip px-4 py-1.5 font-ui text-[10px] font-semibold uppercase tracking-wider transition-colors",
            "bg-p9 text-white hover:bg-p10 focus:outline-none focus:ring-2 focus:ring-p9 focus:ring-offset-1",
            "disabled:opacity-40",
          )}
        >
          {isSending ? t("sending") : t("send")}
        </button>
      </div>

      {/* Discard confirm dialog */}
      <ConfirmDialog
        open={discardDialogOpen}
        title={t("confirm_discard")}
        body="This draft will be permanently deleted. This action cannot be undone."
        confirmLabel="Discard"
        destructive
        onConfirm={handleDiscardConfirm}
        onCancel={() => setDiscardDialogOpen(false)}
      />
    </>
  );
}
