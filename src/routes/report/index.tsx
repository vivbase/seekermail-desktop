// Report route (/report) — decision & risk analytics. T071 lands the risk
// events panel; T089 adds the "AI Activity" tab (E7 audit log). Tab state
// lives in the ui store so switching tabs only re-renders the panel area.
import { useTranslation } from "react-i18next";

import PageBack from "@/components/layout/PageBack";
import { AuditTab } from "@/components/audit/AuditTab";
import { useUi, type ReportTab } from "@/stores/ui";
import { cn } from "@/lib/cn";

import { RiskEventsPanel } from "./RiskEventsPanel";

export default function Report() {
  const { t } = useTranslation(["nav", "common", "audit"]);
  const reportTab = useUi((s) => s.reportTab);
  const setReportTab = useUi((s) => s.setReportTab);

  const tabs: { key: ReportTab; label: string }[] = [
    { key: "risk", label: t("audit:audit_tab_risk") },
    { key: "ai_activity", label: t("audit:audit_tab_label") },
  ];

  return (
    <section className="mx-auto w-full max-w-3xl px-8 py-10">
      <PageBack to="/" labelKey="back_to_dashboard" />
      <p className="section-label mb-2">{t("nav:nav_section_intelligence")}</p>
      <h1 className="font-display text-4xl italic text-p10">{t("nav:nav_report")}</h1>
      <p className="mt-3 font-body text-p8">{t("common:report_desc")}</p>

      {/* Tab bar (T089) */}
      <div role="tablist" aria-label={t("nav:nav_report")} className="mt-6 flex items-center gap-2">
        {tabs.map((tab) => {
          const active = reportTab === tab.key;
          return (
            <button
              key={tab.key}
              type="button"
              role="tab"
              aria-selected={active}
              onClick={() => setReportTab(tab.key)}
              className={cn(
                "rounded-chip px-3 py-1.5 font-ui text-[10px] font-semibold uppercase tracking-wider transition-colors",
                "focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
                active ? "bg-p9 text-white" : "border border-divider text-p8 hover:bg-p4",
              )}
            >
              {tab.label}
            </button>
          );
        })}
      </div>

      <div className="mt-6">{reportTab === "risk" ? <RiskEventsPanel /> : <AuditTab />}</div>
    </section>
  );
}
