// Hand-written mirrors of the Rust AI-settings DTOs (dev/02 §Module H, dev/01
// §account_ai_settings; backend lands with T059's command surface). These types
// exist so T073's /agents UI is fully typed today; once `pnpm gen:types` emits
// them into `@shared/bindings`, delete this file and import from there instead
// (the field shapes below follow the generated-bindings conventions: camelCase,
// `| null` for partial-update fields).

/** Authorization tier: 1 = Manual Only (E1), 2 = Semi-Auto (E2), 3 = Full Auto (E3). */
export type AuthLevel = 1 | 2 | 3;

/** BYO-AI provider slug (dev/02 §1). `none` means no provider configured. */
export type AiProvider = "openai" | "anthropic" | "ollama" | "local_onnx" | "none";

/** Row DTO for `account_ai_settings` (dev/01). `auth_level` mirrors `accounts.auth_level`. */
export type AccountAiSettings = {
  accountId: string;
  authLevel: number;
  aiProvider: AiProvider;
  aiModel: string | null;
  aiBaseUrl: string | null;
  t1Enabled: boolean;
  t2Enabled: boolean;
  t3Enabled: boolean;
  t4Enabled: boolean;
  t5Enabled: boolean;
  t6Enabled: boolean;
  dailyQueryLimit: number;
  e3WhitelistOnly: boolean;
  e3MinHistory: number;
  styleSamplesCount: number;
  updatedAt: number;
};

/**
 * Partial-update params for `update_account_ai_settings` (dev/02 §Module H).
 * `null` means "leave unchanged", matching `UpdateAccountParams` in the
 * generated bindings. `aiApiKey` is written to the Keychain, never the DB.
 */
export type UpdateAiSettingsParams = {
  authLevel: number | null;
  aiProvider: AiProvider | null;
  aiModel: string | null;
  aiApiKey: string | null;
  aiBaseUrl: string | null;
  t1Enabled: boolean | null;
  t2Enabled: boolean | null;
  t3Enabled: boolean | null;
  t4Enabled: boolean | null;
  t5Enabled: boolean | null;
  t6Enabled: boolean | null;
  dailyQueryLimit: number | null;
  e3WhitelistOnly: boolean | null;
  e3MinHistory: number | null;
};

// ── T068 provider-config DTO mirrors (dev/02 §Module H, F_F1/F_F2) ───────────
// Same convention as above: hand-written mirrors of the Rust wire types in
// `src-tauri/src/types.rs`; delete once `pnpm gen:types` emits them.

/** Input to `verify_ai_provider` — probe a key/endpoint without saving. */
export type VerifyAiProviderParams = {
  provider: AiProvider;
  model: string;
  apiKey: string | null;
  baseUrl: string | null;
};

/** In-band probe result (09 §2): failures resolve with `ok: false`, never throw. */
export type VerifyAiProviderResult = {
  ok: boolean;
  modelName: string | null;
  errorMessage: string | null;
};

/** One reachable local AI endpoint found by `scan_local_providers` (F_F2 §3). */
export type LocalProviderEndpoint = {
  baseUrl: string;
  provider: AiProvider;
};

/** One model installed on a local Ollama daemon (`list_ollama_models`, F_F2 §4.3). */
export type OllamaModelEntry = {
  name: string;
  sizeBytes: number;
  parameterSize: string | null;
  quantization: string | null;
};

/**
 * Provider summary row (`list_configured_providers`, T068 §3). Key material is
 * structurally absent — the backend projects only key-free columns (ADR-0004).
 */
export type ConfiguredProviderInfo = {
  accountId: string;
  email: string;
  displayName: string;
  colorToken: string;
  provider: AiProvider;
  model: string | null;
  baseUrl: string | null;
  authLevel: number;
  isLocal: boolean;
  available: boolean;
  updatedAt: number;
};

/** An `UpdateAiSettingsParams` with every field "unchanged" — spread and override. */
export const EMPTY_AI_SETTINGS_PATCH: UpdateAiSettingsParams = {
  authLevel: null,
  aiProvider: null,
  aiModel: null,
  aiApiKey: null,
  aiBaseUrl: null,
  t1Enabled: null,
  t2Enabled: null,
  t3Enabled: null,
  t4Enabled: null,
  t5Enabled: null,
  t6Enabled: null,
  dailyQueryLimit: null,
  e3WhitelistOnly: null,
  e3MinHistory: null,
};
