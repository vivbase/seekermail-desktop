// T4 risk alert banner (T071 §3.2). NON-DISMISSABLE by design: it renders no
// close button and stays until the underlying risk_events row leaves `open`
// (root CLAUDE.md T4 rule, dev/07 §9, AI_MODES_DESIGN §8.1). The only user
// action is "Mark Resolved", which calls `resolve_risk_event`; the riskEvents
// invalidation then removes the banner. Do NOT add a dismiss/close affordance —
// this is a product safety requirement, not a styling choice.
import { useTranslation } from "react-i18next";

import type { RiskEvent } from "@/ipc/legal";
import { useResolveRiskEvent } from "@/ipc/queries/risk";

interface RiskAlertBannerProps {
  event: RiskEvent;
}

export function RiskAlertBanner({ event }: RiskAlertBannerProps) {
  const { t } = useTranslation("legal");
  const resolve = useResolveRiskEvent();

  return (
    <div
      role="alert"
      className="flex items-start gap-3 rounded-card border border-terra p-4"
      style={{
        background: "color-mix(in srgb, var(--terra) 14%, transparent)",
      }}
    >
      <svg
        width="18"
        height="18"
        viewBox="0 0 16 16"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        aria-hidden="true"
        className="mt-0.5 shrink-0 text-terra"
      >
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          d="M8 1.8 14.8 13.5H1.2L8 1.8ZM8 6.2v3.3M8 11.6v.01"
        />
      </svg>

      <div className="min-w-0 flex-1">
        <p className="font-ui text-xs font-semibold uppercase tracking-wider text-terra">
          {t("legal_t4_banner_title")}
        </p>
        <p className="mt-1 font-body text-sm leading-relaxed text-p10">{event.description}</p>
      </div>

      {/* The single allowed action — resolving, never dismissing (T4 rule). */}
      <button
        type="button"
        onClick={() => resolve.mutate({ id: event.id, status: "resolved" })}
        disabled={resolve.isPending}
        className="shrink-0 rounded-chip bg-terra px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white transition-colors hover:bg-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9 disabled:cursor-not-allowed disabled:opacity-50"
      >
        {resolve.isPending ? t("legal_resolving") : t("legal_resolve_btn")}
      </button>
    </div>
  );
}
