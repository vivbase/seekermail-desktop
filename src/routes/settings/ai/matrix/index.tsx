// Settings → AI Providers → Assignment Matrix (T066, F_F4). Joins the
// per-account matrices into a capability × account grid with a fine/simplified
// mode toggle, a reset-to-defaults action, the batch-operations toolbar, and
// the non-blocking advisory warnings returned by `update_provider_matrix`
// (F_F4 §4.5 — amber notices that never block a save).
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";

import {
  ALL_CAPABILITIES,
  matrixCellOf,
  withMatrixCell,
  withoutMatrixCell,
  type BatchMatrixUpdate,
  type Capability,
  type CapabilityMatrix,
  type MatrixCell as MatrixCellValue,
  type MatrixWarning,
} from "@/ipc/aiMatrix";
import {
  useBatchUpdateProviderMatrix,
  useProviderMatrices,
  useResetProviderMatrix,
  useUpdateProviderMatrix,
} from "@/ipc/queries/aiMatrix";
import { useAccounts } from "@/ipc/queries/accounts";
import { useConfiguredProviders } from "@/ipc/queries/aiProviders";
import { cn } from "@/lib/cn";
import { errorText } from "./MatrixCell";
import MatrixGrid, { CAPABILITY_KEY, type MatrixMode } from "./MatrixGrid";
import MatrixToolbar from "./MatrixToolbar";

const TOAST_DURATION_MS = 2800;

/** Backend warning `code` → localized copy (falls back to the wire message). */
const WARNING_KEY: Record<string, string> = {
  small_local_model: "matrix_warning_small_model",
  high_cost_cloud: "matrix_warning_expensive_e4",
  style_history_to_cloud: "matrix_warning_cloud_style",
};

export default function ProviderMatrixPage() {
  const { t } = useTranslation("aiMatrix");
  const { data: accounts, isLoading: accountsLoading, isError: accountsError } = useAccounts();
  const { data: providers } = useConfiguredProviders();
  const updateMatrix = useUpdateProviderMatrix();
  const resetMatrix = useResetProviderMatrix();
  const batchUpdate = useBatchUpdateProviderMatrix();

  const activeAccounts = useMemo(
    () => (accounts ?? []).filter((account) => account.isActive),
    [accounts],
  );
  const accountIds = useMemo(() => activeAccounts.map((a) => a.id), [activeAccounts]);
  const matrixResults = useProviderMatrices(accountIds);

  const [mode, setMode] = useState<MatrixMode>("fine");
  const [warningsByAccount, setWarningsByAccount] = useState<Record<string, MatrixWarning[]>>({});
  const [actionError, setActionError] = useState<string | null>(null);
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

  const matricesLoading = accountsLoading || matrixResults.some((r) => r.isLoading);
  const matricesError = accountsError || matrixResults.some((r) => r.isError);

  const matrices: Record<string, CapabilityMatrix | undefined> = {};
  accountIds.forEach((id, index) => {
    matrices[id] = matrixResults[index]?.data;
  });

  const matrixFor = (accountId: string): CapabilityMatrix => matrices[accountId] ?? { entries: [] };

  const warningText = (warning: MatrixWarning) => {
    const key = WARNING_KEY[warning.code];
    return key ? t(key) : warning.message;
  };

  // Localized warning text per cell, for the amber border + tooltip.
  const warningTexts: Record<string, string> = {};
  for (const [accountId, warnings] of Object.entries(warningsByAccount)) {
    for (const warning of warnings) {
      warningTexts[`${accountId}:${warning.capability}`] = warningText(warning);
    }
  }

  const warningItems = Object.entries(warningsByAccount).flatMap(([accountId, warnings]) => {
    const email = activeAccounts.find((a) => a.id === accountId)?.email ?? accountId;
    return warnings.map((warning) => ({
      key: `${accountId}:${warning.capability}:${warning.code}`,
      email,
      capabilityLabel: t(CAPABILITY_KEY[warning.capability]),
      text: warningText(warning),
    }));
  });

  const runAction = async (action: () => Promise<void>, successMessage: string) => {
    setActionError(null);
    try {
      await action();
      showToast(successMessage);
    } catch (e) {
      setActionError(errorText(e, t("matrix_save_failed")));
    }
  };

  const saveCell = async (accountId: string, capability: Capability, cell: MatrixCellValue) => {
    if (mode === "simplified") {
      // Simplified view shares one configuration: write the same cell to every
      // account in one batch (F_F4 §5 — overwrites account-specific settings).
      const updates: BatchMatrixUpdate[] = activeAccounts.map((account) => ({
        accountId: account.id,
        capability,
        cell,
      }));
      await batchUpdate.mutateAsync(updates);
      setWarningsByAccount({});
      showToast(t("matrix_saved_toast"));
      return;
    }
    const next = withMatrixCell(matrixFor(accountId), capability, cell);
    const warnings = await updateMatrix.mutateAsync({ accountId, matrix: next });
    setWarningsByAccount((prev) => ({ ...prev, [accountId]: warnings }));
    showToast(t("matrix_saved_toast"));
  };

  const clearCell = async (accountId: string, capability: Capability) => {
    const next = withoutMatrixCell(matrixFor(accountId), capability);
    const warnings = await updateMatrix.mutateAsync({ accountId, matrix: next });
    setWarningsByAccount((prev) => ({ ...prev, [accountId]: warnings }));
    showToast(t("matrix_saved_toast"));
  };

  const resetAll = () =>
    runAction(async () => {
      for (const account of activeAccounts) {
        await resetMatrix.mutateAsync(account.id);
      }
      setWarningsByAccount({});
    }, t("matrix_reset_toast"));

  const copyRow = (capability: Capability) => {
    // F_F4 §4.3: the first non-empty cell in account order is the source.
    const source = activeAccounts
      .map((account) => matrixCellOf(matrices[account.id], capability))
      .find((cell): cell is MatrixCellValue => cell !== null);
    if (!source) {
      showToast(t("matrix_batch_nothing_to_copy"));
      return;
    }
    void runAction(async () => {
      await batchUpdate.mutateAsync(
        activeAccounts.map((account) => ({ accountId: account.id, capability, cell: source })),
      );
    }, t("matrix_batch_applied_toast"));
  };

  const copyColumn = (accountId: string) => {
    // Flatten the column: the account's first non-empty cell (row order) is
    // applied to all of that account's capabilities.
    const matrix = matrices[accountId];
    const source = ALL_CAPABILITIES.map((capability) => matrixCellOf(matrix, capability)).find(
      (cell): cell is MatrixCellValue => cell !== null,
    );
    if (!source) {
      showToast(t("matrix_batch_nothing_to_copy"));
      return;
    }
    void runAction(async () => {
      await batchUpdate.mutateAsync(
        ALL_CAPABILITIES.map((capability) => ({ accountId, capability, cell: source })),
      );
    }, t("matrix_batch_applied_toast"));
  };

  const switchRiskToLocal = () => {
    const local = (providers ?? []).find((p) => p.isLocal && p.available);
    if (!local) {
      showToast(t("matrix_no_local_provider"));
      return;
    }
    const cell: MatrixCellValue = {
      primary: { provider: local.provider, model: local.model ?? "", baseUrl: local.baseUrl },
      backups: [],
    };
    void runAction(async () => {
      await batchUpdate.mutateAsync(
        activeAccounts.map((account) => ({
          accountId: account.id,
          capability: "RiskReason" as const,
          cell,
        })),
      );
    }, t("matrix_batch_applied_toast"));
  };

  const busy = batchUpdate.isPending || resetMatrix.isPending;
  const modeButtonClass = (active: boolean) =>
    cn(
      "px-3 py-1.5 font-ui text-xs uppercase tracking-wider transition-colors",
      active ? "bg-p9 text-white" : "text-p9 hover:bg-p4",
    );

  return (
    <div className="max-w-4xl space-y-5">
      <div>
        <Link
          to="/settings/ai"
          className="font-ui text-[10px] uppercase tracking-wider text-p8 transition-colors hover:text-p10"
        >
          {t("matrix_back")}
        </Link>
        <p className="section-label mt-2">{t("matrix_page_title")}</p>
        <p className="mt-2 font-body text-sm leading-relaxed text-p8">
          {t("matrix_page_subtitle")}
        </p>
      </div>

      <div className="flex flex-wrap items-center gap-3">
        <div
          role="group"
          aria-label={t("matrix_mode_label")}
          className="flex overflow-hidden rounded-chip border border-divider"
        >
          <button
            type="button"
            aria-pressed={mode === "fine"}
            onClick={() => setMode("fine")}
            className={modeButtonClass(mode === "fine")}
          >
            {t("matrix_mode_fine")}
          </button>
          <button
            type="button"
            aria-pressed={mode === "simplified"}
            onClick={() => setMode("simplified")}
            className={modeButtonClass(mode === "simplified")}
          >
            {t("matrix_mode_simplified")}
          </button>
        </div>
        <button
          type="button"
          onClick={resetAll}
          disabled={busy || activeAccounts.length === 0}
          className="rounded-chip border border-divider px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p9 transition-colors hover:bg-p4 disabled:opacity-40"
        >
          {resetMatrix.isPending ? t("matrix_resetting") : t("matrix_reset_defaults")}
        </button>
        <details className="ms-auto">
          <summary className="cursor-pointer font-ui text-[10px] uppercase tracking-wider text-p8 transition-colors hover:text-p10">
            {t("matrix_stats_label")}
          </summary>
          <p className="mt-1 max-w-56 font-body text-xs text-p7">{t("matrix_stats_empty")}</p>
        </details>
      </div>

      {mode === "simplified" && (
        <div className="rounded-card border border-amber bg-surface p-3" role="note">
          <p className="font-body text-xs text-p9">{t("matrix_simplified_overwrite")}</p>
        </div>
      )}

      {actionError && (
        <p role="alert" className="font-body text-sm text-red">
          {actionError}
        </p>
      )}

      {warningItems.length > 0 && (
        <div role="status" className="rounded-card border border-amber bg-surface p-4">
          <p className="section-label">{t("matrix_warnings_label")}</p>
          <ul className="mt-2 space-y-1">
            {warningItems.map((item) => (
              <li key={item.key} className="font-body text-xs text-p9">
                <span className="font-mono text-p8">{item.email}</span> · {item.capabilityLabel} —{" "}
                <span>{item.text}</span>
              </li>
            ))}
          </ul>
        </div>
      )}

      {matricesLoading && (
        <div className="rounded-card border border-divider bg-surface p-5">
          <p className="font-body text-sm text-p7">{t("matrix_loading")}</p>
        </div>
      )}
      {!matricesLoading && matricesError && (
        <div role="alert" className="rounded-card border border-red bg-surface p-5">
          <p className="font-body text-sm text-red">{t("matrix_load_error")}</p>
        </div>
      )}
      {!matricesLoading && !matricesError && activeAccounts.length === 0 && (
        <div className="rounded-card border border-divider bg-surface p-5">
          <p className="font-body text-sm text-p7">{t("matrix_empty_accounts")}</p>
        </div>
      )}

      {!matricesLoading && !matricesError && activeAccounts.length > 0 && (
        <>
          <div className="rounded-card border border-divider bg-surface p-4 shadow-card">
            <MatrixGrid
              accounts={activeAccounts}
              mode={mode}
              matrices={matrices}
              warningTexts={warningTexts}
              providers={providers ?? []}
              onSaveCell={saveCell}
              onClearCell={clearCell}
            />
          </div>
          {mode === "fine" && (
            <MatrixToolbar
              accounts={activeAccounts}
              busy={busy}
              onCopyRow={copyRow}
              onCopyColumn={copyColumn}
              onSwitchRiskLocal={switchRiskToLocal}
            />
          )}
        </>
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
