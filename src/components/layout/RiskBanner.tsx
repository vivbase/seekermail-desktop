// T4 risk non-dismissable banner (T100, root CLAUDE.md "Agent-IM": "T4 risk
// warnings are non-dismissable until resolved"). A sticky red banner at the top
// of the shell whenever an open T4 (level-4) risk event exists. It surfaces the
// most recent one with a "+N more" count and links into Pending / the email.
//
// Intentionally NO close button. Per root CLAUDE.md the banner disappears only
// when the risk_events.status changes to resolved/dismissed (via the Pending
// decision card's resolve action); the user cannot dismiss it here.
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

import { useOpenRiskEvents } from "@/ipc/queries/risk";
import { T4_RISK_LEVEL } from "@/ipc/legal";

export default function RiskBanner() {
  const { t } = useTranslation("team");
  const navigate = useNavigate();
  const { data: events = [] } = useOpenRiskEvents();

  const t4 = events.filter((e) => e.riskLevel >= T4_RISK_LEVEL);
  if (t4.length === 0) return null;

  const primary = t4[0];
  if (!primary) return null;
  const extra = t4.length - 1;

  return (
    <div
      role="alert"
      aria-live="assertive"
      aria-atomic="true"
      className="flex items-center gap-3 text-p1"
      style={{ background: "var(--red)", paddingInline: "20px", paddingBlock: "8px" }}
    >
      <span className="shrink-0 font-ui text-xs font-semibold uppercase tracking-wider">
        {t("risk_banner_title")}
      </span>
      <span className="min-w-0 flex-1 truncate font-body text-sm">{primary.description}</span>
      {extra > 0 && (
        <span className="shrink-0 font-mono text-xs">
          {t("risk_banner_more", { count: extra })}
        </span>
      )}
      <button
        type="button"
        onClick={() => navigate("/pending?filter=decision")}
        className="bg-p1/20 hover:bg-p1/30 shrink-0 rounded-chip px-3 py-1 font-ui text-xs font-semibold uppercase tracking-wider focus:outline-none focus-visible:ring-2 focus-visible:ring-p1"
      >
        {t("risk_banner_review")}
      </button>
      {primary.mailId && (
        <button
          type="button"
          onClick={() => navigate(`/mail/${primary.mailId}`)}
          className="shrink-0 font-ui text-xs uppercase tracking-wider underline hover:no-underline focus:outline-none focus-visible:ring-2 focus-visible:ring-p1"
        >
          {t("risk_banner_open_email")}
        </button>
      )}
      {/*
        Intentionally no close button.
        Per root CLAUDE.md: "T4 risk warnings are non-dismissable until resolved."
        The banner clears only when risk_events.status becomes resolved/dismissed
        via resolve_risk_event (triggered from the Pending decision card).
      */}
    </div>
  );
}
