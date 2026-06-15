// Set-primary confirmation dialog (T091, F_I1 §3). Mirrors the Full-Auto dialog
// focus contract (dev/11 §3): focus moves to the safe Cancel action on open, Tab
// cycles inside the dialog, Escape cancels, and focus returns to the opener on
// close. The body explains that the primary agent represents the user in the Team
// channel, so the change is never silent.
import { useEffect, useRef, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";

interface SetPrimaryDialogProps {
  open: boolean;
  /** Display name of the account being promoted (interpolated into the body). */
  accountName: string;
  /** Email of the account being promoted (interpolated into the body). */
  accountEmail: string;
  /** True while the `set_primary_account` mutation is in flight. */
  pending?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export default function SetPrimaryDialog({
  open,
  accountName,
  accountEmail,
  pending = false,
  onConfirm,
  onCancel,
}: SetPrimaryDialogProps) {
  const { t } = useTranslation(["agents", "common"]);
  const dialogRef = useRef<HTMLDivElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const openerRef = useRef<HTMLElement | null>(null);

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
        aria-labelledby="set-primary-dialog-title"
        aria-describedby="set-primary-dialog-body"
      >
        <h2 id="set-primary-dialog-title" className="font-display text-lg italic text-p10">
          {t("agents:set_primary_confirm_title")}
        </h2>
        <p id="set-primary-dialog-body" className="mt-2 font-body text-sm leading-relaxed text-p8">
          {t("agents:set_primary_confirm_body", { name: accountName, email: accountEmail })}
        </p>
        <div className="mt-5 flex justify-end gap-2">
          <button
            ref={cancelRef}
            type="button"
            onClick={onCancel}
            className="rounded-chip px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p8 hover:bg-p4"
          >
            {t("agents:set_primary_cancel")}
          </button>
          <button
            type="button"
            onClick={onConfirm}
            disabled={pending}
            className="rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white hover:bg-p10 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {t("agents:set_primary_confirm_cta")}
          </button>
        </div>
      </div>
    </div>
  );
}
