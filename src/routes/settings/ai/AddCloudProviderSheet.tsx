// Cloud-provider add/edit wizard (T068, F_F1 §3–§4.4). A side sheet with a
// three-step flow: type → credentials → model + accounts. Clicking Continue on
// the credentials step runs the connection probe inline and only advances when
// the provider verifies, so there is no separate manual "test" step.
// Completed steps stay mounted so earlier fields remain editable; changing any
// connection field invalidates a previous test result and snaps back to credentials.
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
import {
  useListCloudModels,
  useUpdateAiSettings,
  useVerifyProvider,
} from "@/ipc/queries/aiProviders";
import { cn } from "@/lib/cn";

/** UI-level provider taxonomy (F_F1 §4.1). Every non-Anthropic vendor rides the
 *  `openai` wire variant with its own base URL (dev/06 §1); Anthropic uses its
 *  native Messages API. The list covers the top global model providers plus
 *  Azure and a generic OpenAI-compatible escape hatch. */
export type CloudProviderType =
  | "openai"
  | "anthropic"
  | "gemini"
  | "xai"
  | "deepseek"
  | "qwen"
  | "mistral"
  | "moonshot"
  | "zhipu"
  | "llama"
  | "azure"
  | "openai_compat";

interface ProviderPreset {
  /** Wire variant the backend adapter dispatches on. */
  wire: AiProvider;
  /** Default API base URL prefilled when the preset is chosen (`""` = use the
   *  adapter default, i.e. the vendor's own host for OpenAI / Anthropic). */
  baseUrl: string;
  /** Whether a base URL must be present before the connection test. */
  baseUrlRequired: boolean;
  /** Curated current model ids so the picker is useful before a key is entered;
   *  the live "Load models" fetch (`GET /v1/models`) supersedes these, and
   *  "Custom" always accepts any id. */
  models: string[];
}

/** One-click presets for the top global LLM providers (June 2026) + Azure + a
 *  generic OpenAI-compatible option. Base URLs match each vendor's documented
 *  endpoint; the version-tolerant join in the Rust adapter handles those that
 *  already carry a version segment (Gemini, Qwen, Zhipu, xAI). */
export const CLOUD_PRESETS: Record<CloudProviderType, ProviderPreset> = {
  openai: {
    wire: "openai",
    baseUrl: "",
    baseUrlRequired: false,
    models: ["gpt-5.5", "gpt-5.4", "gpt-5.4-mini", "gpt-4o"],
  },
  anthropic: {
    wire: "anthropic",
    baseUrl: "",
    baseUrlRequired: false,
    models: ["claude-opus-4-8", "claude-sonnet-4-6", "claude-haiku-4-5"],
  },
  gemini: {
    wire: "openai",
    baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
    baseUrlRequired: true,
    models: ["gemini-3-pro", "gemini-3-flash", "gemini-2.5-pro", "gemini-2.5-flash"],
  },
  xai: {
    wire: "openai",
    baseUrl: "https://api.x.ai/v1",
    baseUrlRequired: true,
    models: ["grok-4.3", "grok-4", "grok-4-fast"],
  },
  deepseek: {
    wire: "openai",
    baseUrl: "https://api.deepseek.com",
    baseUrlRequired: true,
    models: ["deepseek-v4-pro", "deepseek-v4-flash", "deepseek-chat", "deepseek-reasoner"],
  },
  qwen: {
    wire: "openai",
    baseUrl: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
    baseUrlRequired: true,
    models: ["qwen3-max", "qwen3.5-plus", "qwen3.5-flash", "qwen-plus"],
  },
  mistral: {
    wire: "openai",
    baseUrl: "https://api.mistral.ai/v1",
    baseUrlRequired: true,
    models: ["mistral-large-latest", "mistral-medium-latest", "magistral-medium-latest"],
  },
  moonshot: {
    wire: "openai",
    baseUrl: "https://api.moonshot.ai/v1",
    baseUrlRequired: true,
    models: ["kimi-k2.6", "kimi-k2.5", "kimi-k2"],
  },
  zhipu: {
    wire: "openai",
    baseUrl: "https://open.bigmodel.cn/api/paas/v4",
    baseUrlRequired: true,
    models: ["glm-5", "glm-4.6", "glm-4.5"],
  },
  llama: {
    wire: "openai",
    baseUrl: "https://api.llama.com/compat/v1",
    baseUrlRequired: true,
    models: ["llama-4.1-maverick", "llama-4-maverick", "llama-4-scout"],
  },
  azure: {
    wire: "openai",
    baseUrl: "",
    baseUrlRequired: true,
    models: ["gpt-5.5", "gpt-5.4", "gpt-4o"],
  },
  openai_compat: {
    wire: "openai",
    baseUrl: "",
    baseUrlRequired: true,
    models: [],
  },
};

export const CLOUD_TYPES: CloudProviderType[] = [
  "openai",
  "anthropic",
  "gemini",
  "xai",
  "deepseek",
  "qwen",
  "mistral",
  "moonshot",
  "zhipu",
  "llama",
  "azure",
  "openai_compat",
];

/** Sentinel `<select>` value that reveals the free-text custom-model input. */
const CUSTOM_MODEL = "__custom__";

/** Wire provider slug for one UI type. */
export function wireProviderFor(type: CloudProviderType): AiProvider {
  return CLOUD_PRESETS[type].wire;
}

/** Best-effort UI type for an existing row (the wire shape stores only the slug
 *  and base URL). Matches a known vendor by its base URL, else the generic type. */
export function cloudTypeFor(row: ConfiguredProviderInfo): CloudProviderType {
  if (row.provider === "anthropic") return "anthropic";
  if (!row.baseUrl) return "openai";
  const match = CLOUD_TYPES.find(
    (id) => CLOUD_PRESETS[id].baseUrl !== "" && row.baseUrl?.startsWith(CLOUD_PRESETS[id].baseUrl),
  );
  return match ?? "openai_compat";
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
  const listModels = useListCloudModels();

  const [step, setStep] = useState(1);
  const [cloudType, setCloudType] = useState<CloudProviderType>(initial?.cloudType ?? "anthropic");
  const [apiKey, setApiKey] = useState("");
  const [baseUrl, setBaseUrl] = useState(initial?.baseUrl ?? "");
  const [model, setModel] = useState(initial?.model ?? "");
  // Live `GET /v1/models` results once the user runs "Load models"; merged with
  // the curated shortlist in the picker.
  const [fetchedModels, setFetchedModels] = useState<string[] | null>(null);
  const [modelsError, setModelsError] = useState(false);
  // True when the user chose "Custom" — the free-text model-id input is shown.
  const [customModel, setCustomModel] = useState(false);
  const [selectedAccounts, setSelectedAccounts] = useState<Set<string>>(
    () => new Set(initial ? [initial.accountId] : accounts.map((a) => a.id)),
  );
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState(false);
  const [saving, setSaving] = useState(false);

  const baseUrlRequired = CLOUD_PRESETS[cloudType].baseUrlRequired;
  const verified = verify.data?.ok === true;

  /** Any change to the connection inputs invalidates an earlier test result and
   *  snaps back to the credentials step so the probe re-runs on the next Continue. */
  const onConnectionFieldChange = () => {
    if (verify.data || verify.isError) verify.reset();
    if (step > 2) setStep(2);
  };

  /** Switching provider type prefills the vendor's base URL and resets the
   *  model choice (model ids differ per vendor) and the fetched catalog. */
  const selectType = (type: CloudProviderType) => {
    setCloudType(type);
    setBaseUrl(CLOUD_PRESETS[type].baseUrl);
    setModel("");
    setCustomModel(false);
    setFetchedModels(null);
    setModelsError(false);
    onConnectionFieldChange();
  };

  /** Pull the live model catalog (`GET /v1/models`) for the entered key. */
  const loadModels = () => {
    setModelsError(false);
    listModels.mutate(
      {
        provider: wireProviderFor(cloudType),
        apiKey: apiKey.trim() || null,
        baseUrl: baseUrl.trim() || null,
      },
      {
        onSuccess: (ids) => setFetchedModels(ids),
        onError: () => setModelsError(true),
      },
    );
  };

  /** Dropdown change: a real id selects it; the sentinel opens the custom field. */
  const onModelSelect = (value: string) => {
    if (value === CUSTOM_MODEL) {
      setCustomModel(true);
      setModel("");
    } else {
      setCustomModel(false);
      setModel(value);
    }
    onConnectionFieldChange();
  };

  // Curated shortlist + any live-fetched ids, deduped. A pre-filled (edited)
  // model that is in neither list is prepended so it stays selectable.
  const curatedModels = CLOUD_PRESETS[cloudType].models;
  const modelOptions = Array.from(
    new Set([
      ...(model && !customModel ? [model] : []),
      ...curatedModels,
      ...(fetchedModels ?? []),
    ]),
  );

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

  /** Advance the wizard. On the credentials step (2) this validates the inputs,
   *  runs the connection probe inline, and only advances once the provider
   *  verifies — folding the old manual "Test connection" button into Continue.
   *  A failed probe keeps the user on the credentials step with an inline reason. */
  const handleContinue = async () => {
    if (step === 1) {
      setStep(2);
      return;
    }
    if (!validateDetails()) return;
    try {
      const result = await verify.mutateAsync({
        provider: wireProviderFor(cloudType),
        model: model.trim(),
        apiKey: apiKey.trim() || null,
        baseUrl: baseUrl.trim() || null,
      });
      if (result.ok) setStep(3);
    } catch {
      // The verify hook resolves failures in-band (ok:false); a thrown error is
      // unexpected transport trouble and surfaces via `verify.isError` below.
    }
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

  const failure: { key: string; values?: { message: string } } | null = verify.isError
    ? { key: "ai_test_fail_generic", values: { message: "" } }
    : verify.data && !verify.data.ok
      ? verifyFailureKey(verify.data.errorMessage)
      : null;

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
                onClick={() => selectType(type)}
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
            <div className="mt-3">
              <span className="font-ui text-[10px] uppercase tracking-wider text-p8">
                {t("ai_model_label")}
              </span>
              <div className="mt-1 flex items-center gap-2">
                <select
                  aria-label={t("ai_model_label")}
                  value={customModel ? CUSTOM_MODEL : model}
                  onChange={(e) => onModelSelect(e.target.value)}
                  className="grow rounded-chip border border-divider bg-surface px-3 py-2 font-mono text-sm text-p9"
                >
                  <option value="" disabled>
                    {t("ai_model_placeholder")}
                  </option>
                  {modelOptions.map((id) => (
                    <option key={id} value={id}>
                      {id}
                    </option>
                  ))}
                  <option value={CUSTOM_MODEL}>{t("ai_model_option_custom")}</option>
                </select>
                <button
                  type="button"
                  onClick={loadModels}
                  disabled={listModels.isPending || !apiKey.trim()}
                  className="shrink-0 rounded-chip border border-divider px-3 py-2 font-ui text-xs uppercase tracking-wider text-p9 transition-colors hover:bg-p4 disabled:opacity-40"
                >
                  {t("ai_model_load")}
                </button>
              </div>
              {listModels.isPending && (
                <p role="status" className="mt-1 font-body text-sm text-p8">
                  {t("ai_model_loading")}
                </p>
              )}
              {modelsError && (
                <p role="alert" className="mt-1 font-body text-sm text-red">
                  {t("ai_model_load_error")}
                </p>
              )}
              {customModel && (
                <input
                  type="text"
                  aria-label={t("ai_model_custom_label")}
                  value={model}
                  onChange={(e) => {
                    setModel(e.target.value);
                    onConnectionFieldChange();
                  }}
                  placeholder={t("ai_model_custom_placeholder")}
                  className="mt-2 w-full rounded-chip border border-divider bg-surface px-3 py-2 font-mono text-sm text-p9"
                />
              )}
              {/* Hint sits outside the controls so it never bloats their
                  accessible names (screen readers announce only the field). */}
              <p className="mt-1 font-body text-xs text-p7">{t("ai_model_picker_hint")}</p>
            </div>
          </fieldset>
        )}

        {/* Step 3 — model confirmation + accounts (reached only once verified) */}
        {step >= 3 && (
          <fieldset className="mt-5">
            <legend className="section-label">{t("ai_select_models")}</legend>
            {verified && (
              <p role="status" className="mt-2 font-body text-sm text-green">
                ✓ {t("ai_test_success")}
                {verify.data?.modelName ? ` — ${verify.data.modelName}` : ""}
              </p>
            )}
            <p className="mt-2 font-body text-sm text-p9">
              {t("ai_model_label")}: <span className="font-mono text-p10">{model}</span>
            </p>
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

        {/* Inline connection-probe status for the Continue-driven test (step 2). */}
        {verify.isPending && (
          <p role="status" className="mt-3 font-body text-sm text-p8">
            {t("ai_test_running")}
          </p>
        )}
        {failure && (
          <p role="alert" className="mt-3 font-body text-sm text-red">
            {t(failure.key, failure.values)}
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
          {step < 3 && (
            <button
              type="button"
              disabled={step === 2 && verify.isPending}
              onClick={() => void handleContinue()}
              className="rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white transition-colors hover:bg-p10 disabled:cursor-not-allowed disabled:opacity-40"
            >
              {t("ai_step_next")}
            </button>
          )}
          {step === 3 && (
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
