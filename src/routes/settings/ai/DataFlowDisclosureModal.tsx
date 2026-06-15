// Data-flow disclosure modal (T064, dev/06 §8). Shown once before the first
// cloud recommended provider is authorized; the user's confirmation is
// recorded in `app_settings["ai.disclosure_confirmed_at"]` and the backend
// `begin_recommended_oauth` gate refuses cloud grants until it exists — the
// modal is non-bypassable by construction, not just by UI.
//
// Deliberately reusable: T069's data-flow panel links to the same concepts,
// and the F1 manual-key path (T068) can mount this component before a cloud
// provider is saved.
import { useTranslation } from "react-i18next";

export type DataFlowDisclosureModalProps = {
  /** Display name of the cloud provider about to be authorized. */
  providerName: string;
  /** The user explicitly confirmed — record it, then continue the flow. */
  onConfirm: () => void;
  /** The user backed out — return to the previous step, nothing recorded. */
  onCancel: () => void;
};

/**
 * Non-bypassable disclosure (dev/06 §8): no overlay-click dismiss, no close
 * icon, no Escape handler. The only ways out are the explicit Cancel (which
 * aborts the cloud setup) and "I Understand" (which records the confirmation).
 */
export default function DataFlowDisclosureModal({
  providerName,
  onConfirm,
  onCancel,
}: DataFlowDisclosureModalProps) {
  const { t } = useTranslation("aiSetup");

  return (
    <div
      className="bg-p10/40 fixed inset-0 z-50 flex items-center justify-center p-4"
      role="presentation"
    >
      <div
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="ai-disclosure-title"
        className="w-full max-w-md rounded-card border border-divider bg-surface p-5 shadow-card"
      >
        <p id="ai-disclosure-title" className="section-label">
          {t("ai_setup_disclosure_title")}
        </p>
        <p className="mt-3 font-body text-sm leading-relaxed text-p9">
          {t("ai_setup_disclosure_body", { provider: providerName })}
        </p>
        <ul className="mt-3 space-y-2">
          {(
            [
              "ai_setup_disclosure_point_direct",
              "ai_setup_disclosure_point_no_train",
              "ai_setup_disclosure_point_revoke",
            ] as const
          ).map((key) => (
            <li key={key} className="flex gap-2 font-body text-xs leading-relaxed text-p8">
              <span aria-hidden className="mt-0.5 text-green">
                •
              </span>
              <span>{t(key)}</span>
            </li>
          ))}
        </ul>
        <div className="mt-5 flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="rounded-chip border border-divider px-3 py-1.5 font-ui text-xs font-medium text-p8 hover:text-p9"
          >
            {t("ai_setup_cancel")}
          </button>
          <button
            type="button"
            onClick={onConfirm}
            className="rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs font-semibold text-p1 hover:bg-p10"
          >
            {t("ai_setup_disclosure_confirm")}
          </button>
        </div>
      </div>
    </div>
  );
}
