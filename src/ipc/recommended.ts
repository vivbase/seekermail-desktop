// Hand-written mirrors of the Rust F3 recommended-provider DTOs (T064,
// `src-tauri/src/ai/recommended.rs`). These exist so the setup wizard is fully
// typed today; once `pnpm gen:types` exports the T064 command surface into
// `@shared/bindings`, delete this file and import from there instead (shapes
// follow the generated-bindings conventions: camelCase, `| null` optionals).
import type { AiProvider } from "./aiSettings";

/** Recommendation tier slug (F_F3 §3 step 2). */
export type RecommendedTier = "balanced" | "high_quality";

/** One wizard tier card (F_F3 §4.1 + §4.4). Endpoint URLs never cross the wire. */
export type RecommendedProviderInfo = {
  tier: RecommendedTier;
  provider: AiProvider;
  model: string;
  displayName: string;
  monthlyCostMinUsd: number;
  monthlyCostMaxUsd: number;
  tokensPerReplyEstimate: number;
  /** `false` = the wizard must use the authorization-code paste fallback. */
  oauthSupported: boolean;
};

/** `begin_recommended_oauth` result: keep `state` for the completion call. */
export type BeginRecommendedOAuthResult = {
  state: string;
  authorizeUrl: string;
};

/** `complete_recommended_oauth` result — in-band like `verify_ai_provider`. */
export type CompleteRecommendedOAuthResult = {
  ok: boolean;
  providerName: string;
  modelName: string | null;
  errorMessage: string | null;
};

/** Setup snapshot: disclosure / conservative quota / first-auth timestamps. */
export type AiSetupStatus = {
  disclosureConfirmedAt: number | null;
  conservativeQuotaUntil: number | null;
  firstAuthAt: number | null;
};

/** Payload of the `oauth:callback` Tauri event the deep-link handler emits. */
export type RecommendedOAuthCallback = {
  code: string;
  state: string;
};
