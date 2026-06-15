// One configured-provider row (T068, F_F1 §5). Shows the account identity with
// its color token, the provider type + model, the "🔒 Local" badge for local
// providers (F_F2 §4.5), and a status badge that updates after a retest.
// Retesting is only offered for local providers: the frontend never holds a
// saved cloud key (ADR-0004), so a cloud probe from here could not include it —
// cloud rows re-verify inside the Edit sheet instead.
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { EMPTY_AI_SETTINGS_PATCH, type ConfiguredProviderInfo } from "@/ipc/aiSettings";
import { useUpdateAiSettings, useVerifyProvider } from "@/ipc/queries/aiProviders";
import { accountColorClass, type AccountColorToken } from "@/lib/accountColor";
import { cn } from "@/lib/cn";
import { verifyFailureKey } from "./AddCloudProviderSheet";

type RowStatus = "active" | "verified" | "auth_failed" | "unreachable" | "unavailable";

const STATUS_CLASS: Record<RowStatus, string> = {
  active: "text-p8 border-divider",
  verified: "text-green border-green",
  auth_failed: "text-red border-red",
  unreachable: "text-red border-red",
  unavailable: "text-amber border-amber",
};

interface ProviderListItemProps {
  provider: ConfiguredProviderInfo;
  onEdit: (provider: ConfiguredProviderInfo) => void;
  /** Bubble the removal toast up to the page's single toast region. */
  onRemoved: (message: string) => void;
}

export default function ProviderListItem({ provider, onEdit, onRemoved }: ProviderListItemProps) {
  const { t } = useTranslation("aiProviders");
  const verify = useVerifyProvider();
  const updateAi = useUpdateAiSettings();
  const [confirmingRemove, setConfirmingRemove] = useState(false);

  let status: RowStatus = provider.available ? "active" : "unavailable";
  if (verify.data) {
    if (verify.data.ok) status = "verified";
    else {
      const failure = verifyFailureKey(verify.data.errorMessage);
      status = failure.key === "ai_test_fail_401" ? "auth_failed" : "unreachable";
    }
  }

  const retest = () => {
    if (!provider.model) return;
    verify.mutate({
      provider: provider.provider,
      model: provider.model,
      apiKey: null,
      baseUrl: provider.baseUrl,
    });
  };

  const remove = () => {
    updateAi.mutate(
      {
        accountId: provider.accountId,
        params: { ...EMPTY_AI_SETTINGS_PATCH, aiProvider: "none" },
      },
      {
        onSuccess: () => {
          setConfirmingRemove(false);
          onRemoved(t("ai_saved_toast"));
        },
      },
    );
  };

  return (
    <li className="rounded-card border border-divider bg-surface p-4 shadow-card">
      <div className="flex items-center gap-3">
        <span
          aria-hidden
          className={cn(
            "flex h-8 w-8 shrink-0 items-center justify-center rounded-avatar font-ui text-xs",
            accountColorClass(provider.colorToken as AccountColorToken),
          )}
        >
          {provider.displayName.charAt(0).toUpperCase()}
        </span>
        <div className="min-w-0 grow">
          <div className="flex items-center gap-2">
            <p className="truncate font-body text-sm font-medium text-p10">
              {provider.displayName}
            </p>
            {provider.isLocal && (
              <span className="shrink-0 rounded-chip border border-green px-2 py-0.5 font-ui text-[10px] uppercase tracking-wider text-green">
                {t("ai_provider_badge_local")}
              </span>
            )}
            <span
              className={cn(
                "shrink-0 rounded-chip border px-2 py-0.5 font-ui text-[10px] uppercase tracking-wider",
                STATUS_CLASS[status],
              )}
            >
              {t(`ai_provider_status_${status}`)}
            </span>
          </div>
          <p className="mt-0.5 truncate font-mono text-xs text-p8">
            {provider.email} · {t(`ai_provider_type_${provider.provider}`)} ·{" "}
            {provider.model ?? t("ai_provider_no_model")}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {provider.isLocal && (
            <button
              type="button"
              onClick={retest}
              disabled={verify.isPending || !provider.model}
              className="rounded-chip border border-divider px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p9 transition-colors hover:bg-p4 disabled:opacity-40"
            >
              {verify.isPending ? t("ai_test_running") : t("ai_action_retest")}
            </button>
          )}
          <button
            type="button"
            onClick={() => onEdit(provider)}
            className="rounded-chip border border-divider px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p9 transition-colors hover:bg-p4"
          >
            {t("ai_action_edit")}
          </button>
          <button
            type="button"
            onClick={() => setConfirmingRemove(true)}
            className="rounded-chip border border-divider px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-red transition-colors hover:bg-p4"
          >
            {t("ai_action_remove")}
          </button>
        </div>
      </div>

      {confirmingRemove && (
        <div
          role="alertdialog"
          aria-modal="true"
          aria-label={t("ai_remove_confirm_title")}
          className="mt-3 rounded-card border border-divider bg-p4 p-3"
        >
          <p className="font-body text-sm font-medium text-p10">{t("ai_remove_confirm_title")}</p>
          <p className="mt-1 font-body text-xs text-p8">
            {t("ai_remove_confirm_body", { name: provider.displayName })}
          </p>
          <div className="mt-3 flex justify-end gap-2">
            <button
              type="button"
              onClick={() => setConfirmingRemove(false)}
              className="rounded-chip px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p8"
            >
              {t("ai_cancel")}
            </button>
            <button
              type="button"
              onClick={remove}
              disabled={updateAi.isPending}
              className="rounded-chip bg-red px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white disabled:opacity-40"
            >
              {t("ai_remove_confirm_btn")}
            </button>
          </div>
        </div>
      )}
    </li>
  );
}
