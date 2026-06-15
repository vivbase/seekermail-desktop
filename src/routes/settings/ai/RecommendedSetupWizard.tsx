// F3 recommended-provider one-click setup wizard (T064, F_F3 §3–§5).
//
// A useState step machine (no routing): choose path → pick tier → authorize
// in the system browser → connection test → ready. Cloud tiers are gated by
// the non-bypassable data-flow disclosure (dev/06 §8) — the backend refuses
// `begin_recommended_oauth` until the confirmation is recorded, so the modal
// here is the UX face of a server-side rule, not the rule itself.
//
// The OAuth callback arrives either via the `oauth:callback` deep-link event
// or the manual authorization-code paste (F_F3 §6 fallback). Any failure
// lands on one error surface with the spec's three exits: Retry / Use My Key
// / Use a Local Model.
import { useCallback, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

import {
  useAiSetupStatus,
  useBeginRecommendedOAuth,
  useCompleteRecommendedOAuth,
  useConfirmAiDisclosure,
  useOAuthCallbackListener,
  useRecommendedProviders,
} from "@/ipc/queries/ai";
import type {
  CompleteRecommendedOAuthResult,
  RecommendedProviderInfo,
  RecommendedTier,
} from "@/ipc/recommended";

import DataFlowDisclosureModal from "./DataFlowDisclosureModal";

type WizardStep = "choose" | "tiers" | "authorizing" | "testing" | "ready" | "error";

export type RecommendedSetupWizardProps = {
  /** Called when the wizard finishes or is dismissed (Done / Cancel). */
  onClose: () => void;
};

function Spinner() {
  return (
    <span
      aria-hidden
      className="inline-block h-4 w-4 animate-spin rounded-full border-2 border-divider border-t-p9"
    />
  );
}

export default function RecommendedSetupWizard({ onClose }: RecommendedSetupWizardProps) {
  const { t } = useTranslation("aiSetup");
  const navigate = useNavigate();

  const providers = useRecommendedProviders();
  const status = useAiSetupStatus();
  const confirmDisclosure = useConfirmAiDisclosure();
  const beginOAuth = useBeginRecommendedOAuth();
  const completeOAuth = useCompleteRecommendedOAuth();

  const [step, setStep] = useState<WizardStep>("choose");
  const [selected, setSelected] = useState<RecommendedProviderInfo | null>(null);
  const [disclosureFor, setDisclosureFor] = useState<RecommendedProviderInfo | null>(null);
  const [result, setResult] = useState<CompleteRecommendedOAuthResult | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [manualCode, setManualCode] = useState("");
  // The CSRF state nonce from begin — required by the completion call.
  const stateNonceRef = useRef<string | null>(null);

  const fail = useCallback((message: string | null) => {
    setErrorMessage(message);
    setStep("error");
  }, []);

  const startAuthorization = useCallback(
    (info: RecommendedProviderInfo) => {
      setSelected(info);
      beginOAuth.mutate(info.tier, {
        onSuccess: (begun) => {
          stateNonceRef.current = begun.state;
          setStep("authorizing");
        },
        onError: (err) => fail(err instanceof Error ? err.message : null),
      });
    },
    [beginOAuth, fail],
  );

  const selectTier = useCallback(
    (info: RecommendedProviderInfo) => {
      // Cloud tiers require the one-time data-flow disclosure (dev/06 §8);
      // a recorded confirmation skips the modal.
      if (status.data?.disclosureConfirmedAt == null) {
        setDisclosureFor(info);
      } else {
        startAuthorization(info);
      }
    },
    [status.data?.disclosureConfirmedAt, startAuthorization],
  );

  const completeWith = useCallback(
    (code: string, nonce: string | null) => {
      if (!nonce || !code.trim()) return;
      setStep("testing");
      completeOAuth.mutate(
        { stateNonce: nonce, code: code.trim() },
        {
          onSuccess: (res) => {
            if (res.ok) {
              setResult(res);
              setStep("ready");
            } else {
              fail(res.errorMessage);
            }
          },
          onError: (err) => fail(err instanceof Error ? err.message : null),
        },
      );
    },
    [completeOAuth, fail],
  );

  // Deep-link callback (`oauth:callback`) → complete automatically.
  const onCallback = useCallback(
    (payload: { code: string; state: string }) => {
      if (step === "authorizing") completeWith(payload.code, payload.state);
    },
    [step, completeWith],
  );
  useOAuthCallbackListener(onCallback);

  const retry = useCallback(() => {
    setErrorMessage(null);
    setManualCode("");
    if (selected) {
      startAuthorization(selected);
    } else {
      setStep("tiers");
    }
  }, [selected, startAuthorization]);

  const tierLabel = (tier: RecommendedTier) =>
    tier === "balanced" ? t("ai_setup_balanced") : t("ai_setup_high_quality");
  const tierDesc = (tier: RecommendedTier) =>
    tier === "balanced" ? t("ai_setup_balanced_desc") : t("ai_setup_high_quality_desc");

  return (
    <div className="max-w-xl space-y-5">
      <p className="section-label">{t("ai_setup_wizard_title")}</p>

      {step === "choose" && (
        <div className="space-y-3">
          <p className="font-body text-sm leading-relaxed text-p9">{t("ai_setup_intro")}</p>
          <div className="grid gap-3 sm:grid-cols-3">
            <button
              type="button"
              onClick={() => setStep("tiers")}
              className="rounded-card border border-divider bg-surface p-4 text-start hover:border-p7"
            >
              <p className="font-ui text-sm font-semibold text-p9">
                {t("ai_setup_use_recommended")}
              </p>
              <p className="mt-2 font-body text-xs leading-relaxed text-p8">
                {t("ai_setup_use_recommended_desc")}
              </p>
            </button>
            <button
              type="button"
              onClick={() => navigate("/settings/ai")}
              className="rounded-card border border-divider bg-surface p-4 text-start hover:border-p7"
            >
              <p className="font-ui text-sm font-semibold text-p9">{t("ai_setup_use_my_key")}</p>
              <p className="mt-2 font-body text-xs leading-relaxed text-p8">
                {t("ai_setup_use_my_key_desc")}
              </p>
            </button>
            <button
              type="button"
              onClick={() => navigate("/settings/ai")}
              className="rounded-card border border-divider bg-surface p-4 text-start hover:border-p7"
            >
              <p className="font-ui text-sm font-semibold text-p9">{t("ai_setup_use_local")}</p>
              <p className="mt-2 font-body text-xs leading-relaxed text-p8">
                {t("ai_setup_use_local_desc")}
              </p>
            </button>
          </div>
          <button type="button" onClick={onClose} className="font-ui text-xs text-p8 hover:text-p9">
            {t("ai_setup_cancel")}
          </button>
        </div>
      )}

      {step === "tiers" && (
        <div className="space-y-3">
          <p className="font-ui text-sm font-medium text-p9">{t("ai_setup_tier_title")}</p>
          <p className="font-body text-xs leading-relaxed text-p8">{t("ai_setup_tier_intro")}</p>
          <div className="grid gap-3 sm:grid-cols-2">
            {(providers.data ?? []).map((info) => (
              <div
                key={info.tier}
                className="flex flex-col rounded-card border border-divider bg-surface p-4"
              >
                <div className="flex items-center justify-between gap-2">
                  <p className="font-ui text-sm font-semibold text-p9">{tierLabel(info.tier)}</p>
                  {info.tier === "balanced" && (
                    <span className="rounded-chip bg-sage px-2 py-0.5 font-ui text-[10px] font-semibold uppercase tracking-wide text-p10">
                      {t("ai_setup_recommended_badge")}
                    </span>
                  )}
                </div>
                <p className="mt-1 font-body text-xs leading-relaxed text-p8">
                  {tierDesc(info.tier)}
                </p>
                <p className="mt-2 font-mono text-xs text-p9">{info.displayName}</p>
                <p className="font-mono text-xs text-p8">{info.model}</p>
                <p className="mt-2 font-body text-xs text-p8">
                  {t("ai_setup_cost_estimate", {
                    min: info.monthlyCostMinUsd,
                    max: info.monthlyCostMaxUsd,
                  })}
                </p>
                <p className="font-body text-xs text-p8">
                  {t("ai_setup_tokens_estimate", { tokens: info.tokensPerReplyEstimate })}
                </p>
                <button
                  type="button"
                  onClick={() => selectTier(info)}
                  className="mt-3 rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs font-semibold text-p1 hover:bg-p10"
                >
                  {t("ai_setup_connect")}
                </button>
              </div>
            ))}
          </div>
          <button
            type="button"
            onClick={() => setStep("choose")}
            className="font-ui text-xs text-p8 hover:text-p9"
          >
            {t("ai_setup_back")}
          </button>
        </div>
      )}

      {step === "authorizing" && (
        <div className="space-y-4 rounded-card border border-divider bg-surface p-5">
          <div className="flex items-center gap-3">
            <Spinner />
            <p className="font-ui text-sm font-medium text-p9">{t("ai_setup_authorizing")}</p>
          </div>
          <p className="font-body text-xs leading-relaxed text-p8">
            {t("ai_setup_authorizing_hint")}
          </p>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              completeWith(manualCode, stateNonceRef.current);
            }}
            className="space-y-2 border-t border-divider pt-4"
          >
            <label
              htmlFor="recommended-auth-code"
              className="block font-ui text-xs font-medium text-p9"
            >
              {t("ai_setup_manual_code_label")}
            </label>
            <p className="font-body text-xs leading-relaxed text-p8">
              {t("ai_setup_manual_code_hint")}
            </p>
            <div className="flex gap-2">
              <input
                id="recommended-auth-code"
                value={manualCode}
                onChange={(e) => setManualCode(e.target.value)}
                className="min-w-0 flex-1 rounded-chip border border-divider bg-surface px-3 py-1.5 font-mono text-xs text-p9"
                autoComplete="off"
                spellCheck={false}
              />
              <button
                type="submit"
                disabled={!manualCode.trim()}
                className="rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs font-semibold text-p1 hover:bg-p10 disabled:opacity-50"
              >
                {t("ai_setup_manual_code_submit")}
              </button>
            </div>
          </form>
          <button type="button" onClick={onClose} className="font-ui text-xs text-p8 hover:text-p9">
            {t("ai_setup_cancel")}
          </button>
        </div>
      )}

      {step === "testing" && (
        <div className="flex items-center gap-3 rounded-card border border-divider bg-surface p-5">
          <Spinner />
          <p className="font-ui text-sm font-medium text-p9">{t("ai_setup_testing")}</p>
        </div>
      )}

      {step === "ready" && (
        <div className="space-y-3 rounded-card border border-divider bg-surface p-5">
          <p className="font-ui text-base font-semibold text-green">{t("ai_setup_ready")}</p>
          <p className="font-body text-sm leading-relaxed text-p9">
            {t("ai_setup_ready_desc", {
              provider: result?.providerName ?? selected?.displayName ?? "",
            })}
          </p>
          {result?.modelName && <p className="font-mono text-xs text-p8">{result.modelName}</p>}
          <p className="border-amber/60 rounded-card border bg-surface px-3 py-2 font-body text-xs leading-relaxed text-p8">
            {t("ai_setup_quota_notice")}
          </p>
          <button
            type="button"
            onClick={onClose}
            className="rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs font-semibold text-p1 hover:bg-p10"
          >
            {t("ai_setup_done")}
          </button>
        </div>
      )}

      {step === "error" && (
        <div className="border-red/50 space-y-3 rounded-card border bg-surface p-5">
          <p className="font-ui text-sm font-semibold text-red">{t("ai_setup_error_title")}</p>
          <p className="font-body text-sm leading-relaxed text-p9">
            {errorMessage ?? t("ai_setup_error_generic")}
          </p>
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              onClick={retry}
              className="rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs font-semibold text-p1 hover:bg-p10"
            >
              {t("ai_setup_error_retry")}
            </button>
            <button
              type="button"
              onClick={() => navigate("/settings/ai")}
              className="rounded-chip border border-divider px-3 py-1.5 font-ui text-xs font-medium text-p8 hover:text-p9"
            >
              {t("ai_setup_use_my_key")}
            </button>
            <button
              type="button"
              onClick={() => navigate("/settings/ai")}
              className="rounded-chip border border-divider px-3 py-1.5 font-ui text-xs font-medium text-p8 hover:text-p9"
            >
              {t("ai_setup_use_local")}
            </button>
          </div>
        </div>
      )}

      {disclosureFor && (
        <DataFlowDisclosureModal
          providerName={disclosureFor.displayName}
          onConfirm={() => {
            const info = disclosureFor;
            setDisclosureFor(null);
            confirmDisclosure.mutate(undefined, {
              onSuccess: () => startAuthorization(info),
              onError: (err) => fail(err instanceof Error ? err.message : null),
            });
          }}
          onCancel={() => setDisclosureFor(null)}
        />
      )}
    </div>
  );
}
