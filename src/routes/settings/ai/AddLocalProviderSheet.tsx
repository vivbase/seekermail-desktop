// Local-provider add/edit wizard (T068, F_F2 §3–§4.3). Four-step flow:
// discover (auto-scan + manual URL) → model list (size / quantization
// metadata, multi-select) → connection test → accounts + save. Local
// providers hold no credentials: nothing here ever touches the key path, and
// mail content never leaves the device (dev/06 §1, ADR-0004).
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { Account } from "@shared/bindings";

import { EMPTY_AI_SETTINGS_PATCH, type OllamaModelEntry } from "@/ipc/aiSettings";
import {
  useListOllamaModels,
  useScanLocalProviders,
  useUpdateAiSettings,
  useVerifyProvider,
} from "@/ipc/queries/aiProviders";
import { cn } from "@/lib/cn";
import { verifyFailureKey } from "./AddCloudProviderSheet";

/** Human-readable weight size (the daemon reports bytes; 0 = unknown). */
export function formatModelSize(sizeBytes: number): string {
  if (sizeBytes <= 0) return "—";
  const gib = sizeBytes / 1024 ** 3;
  if (gib >= 1) return `${gib.toFixed(1)} GB`;
  const mib = sizeBytes / 1024 ** 2;
  return `${Math.round(mib)} MB`;
}

/** Pre-fill for editing an already-configured local account row. */
export interface LocalSheetInitial {
  accountId: string;
  model: string | null;
  baseUrl: string | null;
}

interface AddLocalProviderSheetProps {
  accounts: Account[];
  initial?: LocalSheetInitial;
  onClose: () => void;
  onSaved: (message: string) => void;
}

export default function AddLocalProviderSheet({
  accounts,
  initial,
  onClose,
  onSaved,
}: AddLocalProviderSheetProps) {
  const { t } = useTranslation("aiProviders");
  const scan = useScanLocalProviders();
  const listModels = useListOllamaModels();
  const verify = useVerifyProvider();
  const updateAi = useUpdateAiSettings();

  const [step, setStep] = useState(1);
  const [baseUrl, setBaseUrl] = useState(initial?.baseUrl ?? "");
  const [manualUrl, setManualUrl] = useState("");
  const [models, setModels] = useState<OllamaModelEntry[] | null>(null);
  const [modelsError, setModelsError] = useState(false);
  const [selectedModels, setSelectedModels] = useState<string[]>(
    initial?.model ? [initial.model] : [],
  );
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState(false);
  const [saving, setSaving] = useState(false);
  const [selectedAccounts, setSelectedAccounts] = useState<Set<string>>(
    () => new Set(initial ? [initial.accountId] : accounts.map((a) => a.id)),
  );

  // First selected model is the account default (F_F2 §4.4 hint in the UI).
  const primaryModel = selectedModels[0] ?? "";
  const verified = verify.data?.ok === true;
  const failure =
    verify.data && !verify.data.ok ? verifyFailureKey(verify.data.errorMessage) : null;

  /** A new endpoint invalidates the model list and any test result. */
  const chooseEndpoint = (url: string) => {
    setBaseUrl(url);
    setModels(null);
    setModelsError(false);
    setSelectedModels([]);
    verify.reset();
    if (step > 2) setStep(2);
  };

  const loadModels = () => {
    setModelsError(false);
    listModels.mutate(baseUrl.trim() || null, {
      onSuccess: (entries) => {
        setModels(entries);
        // Keep a pre-filled edit selection only if the daemon still has it.
        setSelectedModels((prev) => prev.filter((name) => entries.some((m) => m.name === name)));
      },
      onError: () => setModelsError(true),
    });
  };

  const toggleModel = (name: string) => {
    verify.reset();
    setSelectedModels((prev) =>
      prev.includes(name) ? prev.filter((m) => m !== name) : [...prev, name],
    );
  };

  const runTest = () => {
    verify.mutate({
      provider: "ollama",
      model: primaryModel,
      apiKey: null,
      baseUrl: baseUrl.trim() || null,
    });
  };

  const toggleAccount = (id: string) => {
    setSelectedAccounts((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const save = async () => {
    if (selectedAccounts.size === 0) {
      setFieldError(t("ai_accounts_required"));
      return;
    }
    setFieldError(null);
    setSaveError(false);
    setSaving(true);
    try {
      for (const accountId of selectedAccounts) {
        await updateAi.mutateAsync({
          accountId,
          params: {
            ...EMPTY_AI_SETTINGS_PATCH,
            aiProvider: "ollama",
            aiModel: primaryModel,
            aiBaseUrl: baseUrl.trim() || null,
          },
        });
      }
      onSaved(t("ai_saved_toast"));
      onClose();
    } catch {
      setSaveError(true);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div
      className="bg-p10/40 fixed inset-0 z-40 flex justify-end"
      role="presentation"
      onClick={onClose}
    >
      <div
        className="h-full w-full max-w-md overflow-y-auto bg-surface p-5 shadow-card"
        role="dialog"
        aria-modal="true"
        aria-label={initial ? t("ai_local_sheet_title_edit") : t("ai_local_sheet_title")}
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="font-display text-xl italic text-p10">
          {initial ? t("ai_local_sheet_title_edit") : t("ai_local_sheet_title")}
        </h2>

        {/* Step 1 — discovery */}
        <div className="mt-5">
          <p className="section-label">{t("ai_local_step_discover_label")}</p>
          <div className="mt-2 flex items-center gap-3">
            <button
              type="button"
              onClick={() => scan.mutate()}
              disabled={scan.isPending}
              className="rounded-chip border border-divider px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p9 transition-colors hover:bg-p4 disabled:opacity-40"
            >
              {t("ai_local_scan")}
            </button>
            {scan.isPending && (
              <p role="status" className="font-body text-sm text-p8">
                {t("ai_local_scanning")}
              </p>
            )}
          </div>

          {scan.data && scan.data.length > 0 && (
            <div className="mt-3" role="radiogroup" aria-label={t("ai_local_endpoint_label")}>
              <p className="font-body text-sm text-green">
                {t("ai_local_found", { count: scan.data.length })}
              </p>
              <div className="mt-2 space-y-1">
                {scan.data.map((endpoint) => (
                  <button
                    key={endpoint.baseUrl}
                    type="button"
                    role="radio"
                    aria-checked={baseUrl === endpoint.baseUrl}
                    onClick={() => chooseEndpoint(endpoint.baseUrl)}
                    className={cn(
                      "block w-full rounded-chip border px-3 py-2 text-start font-mono text-sm transition-colors",
                      baseUrl === endpoint.baseUrl
                        ? "border-p9 bg-p4 text-p10"
                        : "border-divider text-p9 hover:bg-p4",
                    )}
                  >
                    {endpoint.baseUrl}
                  </button>
                ))}
              </div>
            </div>
          )}
          {scan.data && scan.data.length === 0 && (
            <p role="status" className="mt-3 font-body text-sm text-p8">
              {t("ai_local_none_found")}
            </p>
          )}

          <details className="mt-3" open={Boolean(initial?.baseUrl)}>
            <summary className="cursor-pointer font-body text-sm text-p8">
              {t("ai_local_manual_toggle")}
            </summary>
            <div className="mt-2 flex items-end gap-2">
              <label className="block grow">
                <span className="font-ui text-[10px] uppercase tracking-wider text-p8">
                  {t("ai_local_manual_label")}
                </span>
                <input
                  type="text"
                  value={manualUrl}
                  onChange={(e) => setManualUrl(e.target.value)}
                  className="mt-1 w-full rounded-chip border border-divider bg-surface px-3 py-2 font-mono text-sm text-p9"
                />
              </label>
              <button
                type="button"
                disabled={!manualUrl.trim()}
                onClick={() => chooseEndpoint(manualUrl.trim())}
                className="rounded-chip border border-divider px-3 py-2 font-ui text-xs uppercase tracking-wider text-p9 hover:bg-p4 disabled:opacity-40"
              >
                {t("ai_local_manual_use")}
              </button>
            </div>
          </details>

          {baseUrl && (
            <p className="mt-3 font-body text-sm text-p9">
              {t("ai_local_endpoint_label")}: <span className="font-mono">{baseUrl}</span>
            </p>
          )}
        </div>

        {/* Step 2 — model list */}
        {step >= 2 && baseUrl && (
          <div className="mt-5">
            <p className="section-label">{t("ai_local_step_models_label")}</p>
            <div className="mt-2 flex items-center gap-3">
              <button
                type="button"
                onClick={loadModels}
                disabled={listModels.isPending}
                className="rounded-chip border border-divider px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p9 transition-colors hover:bg-p4 disabled:opacity-40"
              >
                {t("ai_local_load_models")}
              </button>
              {listModels.isPending && (
                <p role="status" className="font-body text-sm text-p8">
                  {t("ai_local_loading_models")}
                </p>
              )}
            </div>
            {modelsError && (
              <p role="alert" className="mt-2 font-body text-sm text-red">
                {t("ai_local_models_error")}
              </p>
            )}
            {models && models.length === 0 && (
              <p role="status" className="mt-2 font-body text-sm text-p8">
                {t("ai_local_models_empty")}
              </p>
            )}
            {models && models.length > 0 && (
              <fieldset className="mt-2">
                <legend className="sr-only">{t("ai_local_models_label")}</legend>
                <div className="space-y-1">
                  {models.map((entry) => (
                    <label
                      key={entry.name}
                      className="flex items-center gap-2 rounded-chip border border-divider px-3 py-2"
                    >
                      <input
                        type="checkbox"
                        checked={selectedModels.includes(entry.name)}
                        onChange={() => toggleModel(entry.name)}
                      />
                      <span className="grow font-mono text-sm text-p9">{entry.name}</span>
                      <span className="font-mono text-xs text-p8">
                        {formatModelSize(entry.sizeBytes)}
                        {entry.parameterSize ? ` · ${entry.parameterSize}` : ""}
                        {entry.quantization ? ` · ${entry.quantization}` : ""}
                      </span>
                    </label>
                  ))}
                </div>
                <p className="mt-2 font-body text-xs text-p7">{t("ai_local_primary_hint")}</p>
              </fieldset>
            )}
          </div>
        )}

        {/* Step 3 — connection test */}
        {step >= 3 && (
          <div className="mt-5">
            <p className="section-label">{t("ai_test_step_label")}</p>
            <div className="mt-2 flex items-center gap-3">
              <button
                type="button"
                onClick={runTest}
                disabled={verify.isPending || !primaryModel}
                className="rounded-chip border border-divider px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p9 transition-colors hover:bg-p4 disabled:opacity-40"
              >
                {t("ai_test_connection")}
              </button>
              {verify.isPending && (
                <p role="status" className="font-body text-sm text-p8">
                  {t("ai_test_running")}
                </p>
              )}
              {verified && (
                <p role="status" className="font-body text-sm text-green">
                  ✓ {t("ai_test_success")}
                  {verify.data?.modelName ? ` — ${verify.data.modelName}` : ""}
                </p>
              )}
            </div>
            {failure && (
              <p role="alert" className="mt-2 font-body text-sm text-red">
                {t(failure.key, failure.values)}
              </p>
            )}
          </div>
        )}

        {/* Step 4 — accounts */}
        {step >= 4 && (
          <fieldset className="mt-5">
            <legend className="section-label">{t("ai_accounts_label")}</legend>
            <div className="mt-2 space-y-1">
              {accounts.map((account) => (
                <label key={account.id} className="flex items-center gap-2 py-1">
                  <input
                    type="checkbox"
                    checked={selectedAccounts.has(account.id)}
                    onChange={() => toggleAccount(account.id)}
                  />
                  <span className="font-body text-sm text-p9">{account.displayName}</span>
                  <span className="truncate font-mono text-xs text-p8">{account.email}</span>
                </label>
              ))}
            </div>
          </fieldset>
        )}

        {fieldError && (
          <p role="alert" className="mt-3 rounded-chip bg-p4 px-3 py-2 font-body text-xs text-red">
            {fieldError}
          </p>
        )}
        {saveError && (
          <p role="alert" className="mt-3 rounded-chip bg-p4 px-3 py-2 font-body text-xs text-red">
            {t("ai_save_failed")}
          </p>
        )}

        {/* Wizard navigation */}
        <div className="mt-6 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded-chip px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p8"
          >
            {t("ai_cancel")}
          </button>
          {step > 1 && (
            <button
              type="button"
              onClick={() => setStep(step - 1)}
              className="rounded-chip border border-divider px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p9"
            >
              {t("ai_step_back")}
            </button>
          )}
          {step < 4 && (
            <button
              type="button"
              disabled={
                (step === 1 && !baseUrl) ||
                (step === 2 && selectedModels.length === 0) ||
                (step === 3 && !verified)
              }
              onClick={() => {
                if (step === 2 && selectedModels.length === 0) {
                  setFieldError(t("ai_local_model_required"));
                  return;
                }
                setFieldError(null);
                setStep(step + 1);
              }}
              className="rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white transition-colors hover:bg-p10 disabled:cursor-not-allowed disabled:opacity-40"
            >
              {t("ai_step_next")}
            </button>
          )}
          {step === 4 && (
            <button
              type="button"
              onClick={() => void save()}
              disabled={saving || !primaryModel}
              className="rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white transition-colors hover:bg-p10 disabled:cursor-not-allowed disabled:opacity-40"
            >
              {saving ? t("ai_step_saving") : t("ai_step_save")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
