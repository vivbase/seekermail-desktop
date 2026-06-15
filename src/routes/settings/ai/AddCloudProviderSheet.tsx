// Cloud-provider add/edit wizard (T068, F_F1 §3–§4.4). A side sheet with a
// four-step flow: type → credentials → connection test → model + accounts.
// Completed steps stay mounted so earlier fields remain editable; changing any
// connection field invalidates a previous test result.
//
// Key protection (ADR-0004, F_F1 §4.2): the API key lives only in local form
// state on its way to `update_account_ai_settings` (which writes it to the
// Keychain); the input is cleared the moment save starts and a saved key is
// never echoed back — editing shows a masked placeholder instead.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { Account } from "@shared/bindings";

import {
  EMPTY_AI_SETTINGS_PATCH,
  type AiProvider,
  type ConfiguredProviderInfo,
} from "@/ipc/aiSettings";
import { useUpdateAiSettings, useVerifyProvider } from "@/ipc/queries/aiProviders";
import { cn } from "@/lib/cn";

/** UI-level provider taxonomy (F_F1 §4.1). Azure / Gemini / OpenAI-compatible
 *  vendors ride the `openai` wire variant with a custom base URL (dev/06 §1). */
export type CloudProviderType = "anthropic" | "openai" | "openai_compat" | "azure" | "gemini";

export const CLOUD_TYPES: CloudProviderType[] = [
  "anthropic",
  "openai",
  "openai_compat",
  "azure",
  "gemini",
];

const BASE_URL_REQUIRED: Record<CloudProviderType, boolean> = {
  anthropic: false,
  openai: false,
  openai_compat: true,
  azure: true,
  gemini: true,
};

/** Wire provider slug for one UI type. */
export function wireProviderFor(type: CloudProviderType): AiProvider {
  return type === "anthropic" ? "anthropic" : "openai";
}

/** Best-effort UI type for an existing row (the wire shape stores only the slug). */
export function cloudTypeFor(row: ConfiguredProviderInfo): CloudProviderType {
  if (row.provider === "anthropic") return "anthropic";
  return row.baseUrl ? "openai_compat" : "openai";
}

/**
 * Map the sanitized `VerifyAiProviderResult.errorMessage` (the content-free
 * `ProviderError` rendering) onto a user-facing i18n key (F_F1 §4.3).
 */
export function verifyFailureKey(message: string | null): {
  key: string;
  values?: { message: string };
} {
  if (!message) return { key: "ai_test_fail_generic", values: { message: "" } };
  if (message.includes("auth rejected")) return { key: "ai_test_fail_401" };
  if (message.includes("404")) return { key: "ai_test_fail_404" };
  if (message.includes("unreachable")) return { key: "ai_test_fail_unreachable" };
  if (message.includes("rate limited")) return { key: "ai_test_fail_quota" };
  return { key: "ai_test_fail_generic", values: { message } };
}

function isValidHttpUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return url.protocol === "http:" || url.protocol === "https:";
  } catch {
    return false;
  }
}

/** Pre-fill for editing an already-configured account row. */
export interface CloudSheetInitial {
  accountId: string;
  cloudType: CloudProviderType;
  model: string | null;
  baseUrl: string | null;
  /** True when a key was saved before — the key field then shows the masked hint. */
  hasSavedKey: boolean;
}

interface AddCloudProviderSheetProps {
  accounts: Account[];
  initial?: CloudSheetInitial;
  onClose: () => void;
  onSaved: (message: string) => void;
}

export default function AddCloudProviderSheet({
  accounts,
  initial,
  onClose,
  onSaved,
}: AddCloudProviderSheetProps) {
  const { t } = useTranslation("aiProviders");
  const verify = useVerifyProvider();
  const updateAi = useUpdateAiSettings();

  const [step, setStep] = useState(1);
  const [cloudType, setCloudType] = useState<CloudProviderType>(initial?.cloudType ?? "anthropic");
  const [apiKey, setApiKey] = useState("");
  const [baseUrl, setBaseUrl] = useState(initial?.baseUrl ?? "");
  const [model, setModel] = useState(initial?.model ?? "");
  const [selectedAccounts, setSelectedAccounts] = useState<Set<string>>(
    () => new Set(initial ? [initial.accountId] : accounts.map((a) => a.id)),
  );
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState(false);
  const [saving, setSaving] = useState(false);

  const baseUrlRequired = BASE_URL_REQUIRED[cloudType];
  const verified = verify.data?.ok === true;

  /** Any change to the connection inputs invalidates an earlier test result. */
  const onConnectionFieldChange = () => {
    if (verify.data || verify.isError) verify.reset();
    if (step > 3) setStep(3);
  };

  const validateDetails = (): boolean => {
    if (!apiKey.trim() && !initial?.hasSavedKey) {
      setFieldError(t("ai_key_required"));
      return false;
    }
    if (baseUrlRequired && !isValidHttpUrl(baseUrl.trim())) {
      setFieldError(t("ai_base_url_invalid"));
      return false;
    }
    if (baseUrl.trim() && !isValidHttpUrl(baseUrl.trim())) {
      setFieldError(t("ai_base_url_invalid"));
      return false;
    }
    if (!model.trim()) {
      setFieldError(t("ai_model_required"));
      return false;
    }
    setFieldError(null);
    return true;
  };

  const runTest = () => {
    verify.mutate({
      provider: wireProviderFor(cloudType),
      model: model.trim(),
      apiKey: apiKey.trim() || null,
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
    // ADR-0004: the form copy of the key is dropped the moment submit starts;
    // from here it exists only inside the in-flight mutation payload.
    const transientKey = apiKey.trim();
    setApiKey("");
    try {
      for (const accountId of selectedAccounts) {
        await updateAi.mutateAsync({
          accountId,
          params: {
            ...EMPTY_AI_SETTINGS_PATCH,
            aiProvider: wireProviderFor(cloudType),
            aiModel: model.trim(),
            aiBaseUrl: baseUrl.trim() || null,
            aiApiKey: transientKey || null,
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

  const failure =
    verify.data && !verify.data.ok ? verifyFailureKey(verify.data.errorMessage) : null;

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
        aria-label={initial ? t("ai_cloud_sheet_title_edit") : t("ai_cloud_sheet_title")}
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="font-display text-xl italic text-p10">
          {initial ? t("ai_cloud_sheet_title_edit") : t("ai_cloud_sheet_title")}
        </h2>

        {/* Step 1 — provider type */}
        <fieldset className="mt-5">
          <legend className="section-label">{t("ai_cloud_step_type_label")}</legend>
          <div className="mt-2 grid grid-cols-2 gap-2" role="radiogroup">
            {CLOUD_TYPES.map((type) => (
              <button
                key={type}
                type="button"
                role="radio"
                aria-checked={cloudType === type}
                onClick={() => {
                  setCloudType(type);
                  onConnectionFieldChange();
                }}
                className={cn(
                  "rounded-chip border px-3 py-2 text-start font-ui text-sm transition-colors",
                  cloudType === type
                    ? "border-p9 bg-p4 font-medium text-p10"
                    : "border-divider text-p9 hover:bg-p4",
                )}
              >
                {t(`ai_provider_type_${type}`)}
              </button>
            ))}
          </div>
        </fieldset>

        {/* Step 2 — connection details */}
        {step >= 2 && (
          <fieldset className="mt-5">
            <legend className="section-label">{t("ai_cloud_step_details_label")}</legend>
            <label className="mt-2 block">
              <span className="font-ui text-[10px] uppercase tracking-wider text-p8">
                {t("ai_key_entry_label")}
              </span>
              <input
                type="password"
                autoComplete="off"
                value={apiKey}
                placeholder={initial?.hasSavedKey ? t("ai_key_masked") : undefined}
                onChange={(e) => {
                  setApiKey(e.target.value);
                  onConnectionFieldChange();
                }}
                className="mt-1 w-full rounded-chip border border-divider bg-surface px-3 py-2 font-mono text-sm text-p9"
              />
            </label>
            <label className="mt-3 block">
              <span className="font-ui text-[10px] uppercase tracking-wider text-p8">
                {baseUrlRequired ? t("ai_base_url_label") : t("ai_base_url_optional")}
              </span>
              <input
                type="text"
                value={baseUrl}
                onChange={(e) => {
                  setBaseUrl(e.target.value);
                  onConnectionFieldChange();
                }}
                className="mt-1 w-full rounded-chip border border-divider bg-surface px-3 py-2 font-mono text-sm text-p9"
              />
            </label>
            <label className="mt-3 block">
              <span className="font-ui text-[10px] uppercase tracking-wider text-p8">
                {t("ai_model_label")}
              </span>
              <input
                type="text"
                value={model}
                onChange={(e) => {
                  setModel(e.target.value);
                  onConnectionFieldChange();
                }}
                className="mt-1 w-full rounded-chip border border-divider bg-surface px-3 py-2 font-mono text-sm text-p9"
              />
            </label>
            {/* Hint lives outside the label so it does not bloat the input's
                accessible name (screen readers would otherwise announce it as
                part of the field label). */}
            <p className="mt-1 font-body text-xs text-p7">{t("ai_model_custom_hint")}</p>
          </fieldset>
        )}

        {/* Step 3 — connection test */}
        {step >= 3 && (
          <div className="mt-5">
            <p className="section-label">{t("ai_test_step_label")}</p>
            <div className="mt-2 flex items-center gap-3">
              <button
                type="button"
                onClick={runTest}
                disabled={verify.isPending}
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

        {/* Step 4 — model confirmation + accounts */}
        {step >= 4 && (
          <fieldset className="mt-5">
            <legend className="section-label">{t("ai_select_models")}</legend>
            <label className="mt-2 block">
              <span className="font-ui text-[10px] uppercase tracking-wider text-p8">
                {t("ai_model_label")}
              </span>
              <input
                type="text"
                value={model}
                onChange={(e) => setModel(e.target.value)}
                className="mt-1 w-full rounded-chip border border-divider bg-surface px-3 py-2 font-mono text-sm text-p9"
              />
            </label>
            <p className="section-label mt-4">{t("ai_accounts_label")}</p>
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
              disabled={step === 3 && !verified}
              onClick={() => {
                if (step === 2 && !validateDetails()) return;
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
              disabled={saving || !model.trim()}
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
