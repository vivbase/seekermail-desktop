// Settings → AI Providers (T068, F_F1 §5, F_F2 §3). Replaces the T073 stub:
// entry buttons for the cloud wizard, the local (Ollama) wizard, and the
// recommended-setup flow (mounted at /settings/ai/recommended by the T064
// card), plus the configured-provider list aggregated by
// `list_configured_providers`.
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";

import type { ConfiguredProviderInfo } from "@/ipc/aiSettings";
import { useAccounts } from "@/ipc/queries/accounts";
import { useConfiguredProviders } from "@/ipc/queries/aiProviders";
import AddCloudProviderSheet, { cloudTypeFor } from "./AddCloudProviderSheet";
import AddLocalProviderSheet from "./AddLocalProviderSheet";
import AuthLevelSection from "./AuthLevelSection";
import ProviderListItem from "./ProviderListItem";

const TOAST_DURATION_MS = 2800;

type OpenSheet =
  | { kind: "cloud"; edit?: ConfiguredProviderInfo }
  | { kind: "local"; edit?: ConfiguredProviderInfo }
  | null;

export default function AiProvidersPage() {
  const { t } = useTranslation("aiProviders");
  const { data: providers, isLoading } = useConfiguredProviders();
  const { data: accounts } = useAccounts();

  const [sheet, setSheet] = useState<OpenSheet>(null);
  const [toast, setToast] = useState<string | null>(null);
  const toastTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const showToast = (message: string) => {
    if (toastTimer.current) clearTimeout(toastTimer.current);
    setToast(message);
    toastTimer.current = setTimeout(() => setToast(null), TOAST_DURATION_MS);
  };

  useEffect(() => {
    return () => {
      if (toastTimer.current) clearTimeout(toastTimer.current);
    };
  }, []);

  const openEdit = (provider: ConfiguredProviderInfo) => {
    setSheet(
      provider.isLocal ? { kind: "local", edit: provider } : { kind: "cloud", edit: provider },
    );
  };

  return (
    <div className="max-w-2xl space-y-5">
      <div>
        <p className="section-label">{t("ai_providers_page_title")}</p>
        <p className="mt-2 font-body text-sm leading-relaxed text-p8">
          {t("ai_providers_page_subtitle")}
        </p>
      </div>

      {/* Entry actions */}
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          onClick={() => setSheet({ kind: "cloud" })}
          className="rounded-chip bg-p9 px-4 py-2 font-ui text-xs uppercase tracking-wider text-white transition-colors hover:bg-p10"
        >
          {t("ai_add_cloud")}
        </button>
        <button
          type="button"
          onClick={() => setSheet({ kind: "local" })}
          className="rounded-chip border border-divider px-4 py-2 font-ui text-xs uppercase tracking-wider text-p9 transition-colors hover:bg-p4"
        >
          {t("ai_add_local")}
        </button>
        <Link
          to="/settings/ai/recommended"
          title={t("ai_recommended_setup_desc")}
          className="rounded-chip border border-divider px-4 py-2 font-ui text-xs uppercase tracking-wider text-p9 transition-colors hover:bg-p4"
        >
          {t("ai_recommended_setup")}
        </Link>
        <Link
          to="/settings/ai/matrix"
          title={t("ai_assignment_matrix_desc")}
          className="rounded-chip border border-divider px-4 py-2 font-ui text-xs uppercase tracking-wider text-p9 transition-colors hover:bg-p4"
        >
          {t("ai_assignment_matrix")}
        </Link>
      </div>

      {/* Configured providers */}
      <div>
        <p className="section-label">{t("ai_providers_list_label")}</p>
        {isLoading && (
          <div className="mt-2 rounded-card border border-divider bg-surface p-5">
            <p className="font-body text-sm text-p7">{t("ai_providers_loading")}</p>
          </div>
        )}
        {!isLoading && (providers?.length ?? 0) === 0 && (
          <div className="mt-2 rounded-card border border-divider bg-surface p-5">
            <p className="font-body text-sm text-p7">{t("ai_providers_empty")}</p>
          </div>
        )}
        {(providers?.length ?? 0) > 0 && (
          <ul className="mt-2 space-y-3">
            {providers?.map((provider) => (
              <ProviderListItem
                key={provider.accountId}
                provider={provider}
                onEdit={openEdit}
                onRemoved={showToast}
              />
            ))}
          </ul>
        )}
      </div>

      {/* Per-account authorization levels (T086, F_E3 §4.1) */}
      <AuthLevelSection />

      {sheet?.kind === "cloud" && (
        <AddCloudProviderSheet
          accounts={accounts ?? []}
          initial={
            sheet.edit
              ? {
                  accountId: sheet.edit.accountId,
                  cloudType: cloudTypeFor(sheet.edit),
                  model: sheet.edit.model,
                  baseUrl: sheet.edit.baseUrl,
                  hasSavedKey: true,
                }
              : undefined
          }
          onClose={() => setSheet(null)}
          onSaved={showToast}
        />
      )}
      {sheet?.kind === "local" && (
        <AddLocalProviderSheet
          accounts={accounts ?? []}
          initial={
            sheet.edit
              ? {
                  accountId: sheet.edit.accountId,
                  model: sheet.edit.model,
                  baseUrl: sheet.edit.baseUrl,
                }
              : undefined
          }
          onClose={() => setSheet(null)}
          onSaved={showToast}
        />
      )}

      {toast && (
        <div
          role="status"
          aria-live="polite"
          className="fixed bottom-6 z-50 rounded-card bg-p9 px-4 py-3 font-ui text-sm text-surface shadow-card [inset-inline-end:1.5rem]"
        >
          {toast}
        </div>
      )}
    </div>
  );
}
