// Shared confirm dialog (T017). Renders nothing when closed; the caller owns the
// open state and the confirm/cancel handlers. Token-styled, i18n copy only.
import { useTranslation } from "react-i18next";

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  body: string;
  /** Confirm button label (i18n key already resolved by caller, or default). */
  confirmLabel?: string;
  /** Set when the confirm action is destructive (red accent). */
  destructive?: boolean;
  /** Hide the confirm button (e.g. "you can't delete your only account"). */
  confirmDisabled?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export default function ConfirmDialog({
  open,
  title,
  body,
  confirmLabel,
  destructive = false,
  confirmDisabled = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const { t } = useTranslation();
  if (!open) return null;

  return (
    <div
      className="bg-p10/40 fixed inset-0 z-50 flex items-center justify-center p-4"
      onClick={onCancel}
      role="presentation"
    >
      <div
        className="w-full max-w-md rounded-card bg-surface p-5 shadow-card"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
      >
        <h2 className="font-display text-lg italic text-p10">{title}</h2>
        <p className="mt-2 font-body text-sm text-p8">{body}</p>
        <div className="mt-5 flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="rounded-chip px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p8 hover:bg-p4"
          >
            {t("action_cancel")}
          </button>
          {!confirmDisabled && (
            <button
              type="button"
              onClick={onConfirm}
              className={`rounded-chip px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white ${
                destructive ? "bg-red" : "bg-p9"
              }`}
            >
              {confirmLabel ?? t("action_confirm")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
