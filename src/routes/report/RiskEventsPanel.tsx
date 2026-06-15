// Report-page risk events panel (T071 §3.4). Lists open `risk_events` rows
// with a level color block, description, a link to the source mail, and the
// resolve/dismiss actions. T4 rows (riskLevel = 4) offer ONLY "Resolve" —
// dismiss does not apply to T4 (AI_MODES_DESIGN §8.1, root CLAUDE.md T4 rule).
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";

import { riskEventLevelColorVar, T4_RISK_LEVEL, type RiskEvent } from "@/ipc/legal";
import { useResolveRiskEvent, useRiskEvents } from "@/ipc/queries/risk";
import { formatMailDate } from "@/lib/formatDate";

export function RiskEventsPanel() {
  const { t } = useTranslation(["legal", "common"]);
  const { data: events, isLoading } = useRiskEvents({ status: "open" });

  return (
    <section aria-label={t("legal:legal_report_section_title")}>
      <p className="section-label mb-3">{t("legal:legal_report_section_title")}</p>

      {isLoading && (
        <div className="rounded-card border border-divider bg-surface p-6 shadow-card">
          <p className="font-body text-p7">{t("common:state_loading")}</p>
        </div>
      )}

      {!isLoading && (events ?? []).length === 0 && (
        <div className="rounded-card border border-divider bg-surface p-6 shadow-card">
          <p className="font-body text-p7">{t("legal:legal_no_open_risks")}</p>
        </div>
      )}

      {!isLoading && (events ?? []).length > 0 && (
        <ul role="list" className="flex flex-col gap-3">
          {(events ?? []).map((event) => (
            <RiskEventRow key={event.id} event={event} />
          ))}
        </ul>
      )}
    </section>
  );
}

function RiskEventRow({ event }: { event: RiskEvent }) {
  const { t } = useTranslation("legal");
  const resolve = useResolveRiskEvent();
  const isT4 = event.riskLevel === T4_RISK_LEVEL;

  return (
    <li className="flex items-start gap-3 rounded-card border border-divider bg-surface p-4 shadow-card">
      {/* Level color block */}
      <span
        className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-chip font-ui text-[10px] font-semibold uppercase tracking-wider text-white"
        style={{ background: riskEventLevelColorVar(event.riskLevel) }}
        aria-label={t("legal_report_level_badge", { level: event.riskLevel })}
      >
        {t("legal_report_level_badge", { level: event.riskLevel })}
      </span>

      <div className="min-w-0 flex-1">
        <p className="font-body text-sm leading-relaxed text-p10">{event.description}</p>
        <div className="mt-1.5 flex items-center gap-3">
          <Link
            to={`/mail/${event.mailId}`}
            className="font-ui text-[10px] uppercase tracking-wider text-p9 underline transition-colors hover:text-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
          >
            {t("legal_report_open_mail")}
          </Link>
          <span className="font-mono text-[10px] text-p7">{formatMailDate(event.createdAt)}</span>
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-2">
        {/* Dismiss is never offered for T4 (AI_MODES_DESIGN §8.1). */}
        {!isT4 && (
          <button
            type="button"
            onClick={() => resolve.mutate({ id: event.id, status: "dismissed" })}
            disabled={resolve.isPending}
            className="rounded-chip border border-divider px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p8 transition-colors hover:bg-p4 hover:text-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {t("legal_dismiss_btn")}
          </button>
        )}
        <button
          type="button"
          onClick={() => resolve.mutate({ id: event.id, status: "resolved" })}
          disabled={resolve.isPending}
          className="rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white transition-colors hover:bg-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {resolve.isPending ? t("legal_resolving") : t("legal_resolve_btn")}
        </button>
      </div>
    </li>
  );
}
