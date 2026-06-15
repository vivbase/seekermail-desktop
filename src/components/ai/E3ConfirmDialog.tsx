// Full-Auto enablement confirmation (T086, F_E3 §4.1). Stronger than the
// /agents FullAutoConfirmDialog: lists the three locked risk rules and gates
// the Enable button behind an explicit "I understand the risks" checkbox.
// Accessible dialog per dev/11 §3 — focus trap, Esc cancels, focus returns to
// the opener on close. No external dialog primitive exists in this repo, so
// the trap mirrors the proven FullAutoConfirmDialog implementation.
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";

interface E3ConfirmDialogProps {
  open: boolean;
  /** Account display name shown in the title for context. */
  accountName?: string;
  onConfirm: () => void;
  onCancel: () => void;
}

const RISK_KEYS = [
  "e3_confirm_risk_money",
  "e3_confirm_risk_attachment",
  "e3_confirm_risk_contact",
] as const;

export function E3ConfirmDialog({ open, accountName, onConfirm, onCancel }: E3ConfirmDialogProps) {
  const { t } = useTranslation(["aiDrafts", "common"]);
  const dialogRef = useRef<HTMLDivElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const openerRef = useRef<HTMLElement | null>(null);
  const [acknowledged, setAcknowledged] = useState(false);

  // Reset the checkbox every time the dialog opens — the acknowledgement must
  // be given per enablement, never remembered (F_E3 §4.1).
  useEffect(() => {
    if (open) setAcknowledged(false);
  }, [open]);

  // Focus management (dev/11 §3): remember the opener, focus the safe action,
  // restore on close.
  useEffect(() => {
    if (!open) return;
    openerRef.current = document.activeElement as HTMLElement | null;
    cancelRef.current?.focus();
    return () => openerRef.current?.focus();
  }, [open]);

  if (!open) return null;

  const handleKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      onCancel();
      return;
    }
    if (e.key !== "Tab" || !dialogRef.current) return;
    const focusables = dialogRef.current.querySelectorAll<HTMLElement>(
      "button:not([disabled]), [href], input, select, textarea, [tabindex]:not([tabindex='-1'])",
    );
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    if (!first || !last) return;
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };

  return (
    <div
      className="bg-p10/40 fixed inset-0 z-50 flex items-center justify-center p-4"
      onClick={onCancel}
      role="presentation"
    >
      <div
        ref={dialogRef}
        className="w-full max-w-md rounded-card bg-surface p-5 shadow-card"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={handleKeyDown}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="e3-confirm-title"
        aria-describedby="e3-confirm-body"
      >
        <h2 id="e3-confirm-title" className="font-display text-lg italic text-p10">
          {t("aiDrafts:e3_confirm_title")}
          {accountName ? <span className="text-p8"> — {accountName}</span> : null}
        </h2>
        <p id="e3-confirm-body" className="mt-2 font-body text-sm leading-relaxed text-p8">
          {t("aiDrafts:e3_confirm_body")}
        </p>

        {/* Locked rules — these always hold drafts for review (F_E3 §4.2). */}
        <ul className="mt-3 space-y-1.5">
          {RISK_KEYS.map((key) => (
            <li key={key} className="flex items-start gap-2 font-body text-sm text-p9">
              <span
                aria-hidden="true"
                className="mt-1.5 inline-block h-1.5 w-1.5 shrink-0 rounded-avatar bg-terra"
              />
              {t(`aiDrafts:${key}`)}
            </li>
          ))}
        </ul>

        <label className="mt-4 flex cursor-pointer items-center gap-2">
          <input
            type="checkbox"
            checked={acknowledged}
            onChange={(e) => setAcknowledged(e.target.checked)}
            className="h-4 w-4 cursor-pointer rounded border-divider accent-p9"
          />
          <span className="font-body text-sm text-p10">{t("aiDrafts:e3_confirm_ack")}</span>
        </label>

        <div className="mt-5 flex justify-end gap-2">
          <button
            ref={cancelRef}
            type="button"
            onClick={onCancel}
            className="rounded-chip px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p8 hover:bg-p4 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
          >
            {t("common:action_cancel")}
          </button>
          <button
            type="button"
            onClick={onConfirm}
            disabled={!acknowledged}
            className="rounded-chip bg-green px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white transition-opacity hover:opacity-90 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {t("aiDrafts:e3_confirm_enable")}
          </button>
        </div>
      </div>
    </div>
  );
}

export default E3ConfirmDialog;
