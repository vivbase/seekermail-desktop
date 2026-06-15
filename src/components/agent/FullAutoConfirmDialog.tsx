// Full-Auto confirmation dialog (T073, AI_MODES_DESIGN §7.4). Intercepts the
// switch to auth level 3 BEFORE it is written to form state: Cancel keeps the
// previous level, Confirm lets the caller apply level 3. Implements a focus
// trap per dev/11 §3 — focus moves into the dialog on open, Tab cycles inside
// it, Escape cancels, and focus returns to the opener on close.
import { useEffect, useRef, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";

interface FullAutoConfirmDialogProps {
  open: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export default function FullAutoConfirmDialog({
  open,
  onConfirm,
  onCancel,
}: FullAutoConfirmDialogProps) {
  const { t } = useTranslation(["agents", "common"]);
  const dialogRef = useRef<HTMLDivElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const openerRef = useRef<HTMLElement | null>(null);

  // Focus management: remember the opener, move focus to the safe action, and
  // restore focus when the dialog closes (dev/11 §3).
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
        aria-labelledby="full-auto-dialog-title"
        aria-describedby="full-auto-dialog-body"
      >
        <h2 id="full-auto-dialog-title" className="font-display text-lg italic text-p10">
          {t("agents:agents_full_auto_dialog_title")}
        </h2>
        <p id="full-auto-dialog-body" className="mt-2 font-body text-sm leading-relaxed text-p8">
          {t("agents:agents_full_auto_dialog_body")}
        </p>
        <div className="mt-5 flex justify-end gap-2">
          <button
            ref={cancelRef}
            type="button"
            onClick={onCancel}
            className="rounded-chip px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p8 hover:bg-p4"
          >
            {t("common:action_cancel")}
          </button>
          <button
            type="button"
            onClick={onConfirm}
            className="rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white hover:bg-p10"
          >
            {t("agents:agents_full_auto_confirm")}
          </button>
        </div>
      </div>
    </div>
  );
}
