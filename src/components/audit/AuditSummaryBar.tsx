// E7 summary bar (T089 §3, F_E7 §4.6): four stat blocks — Total Events,
// Auto-Sent, Success Rate, Tokens Used. Numbers in --fm, labels in --fu;
// pulse-skeleton placeholders while the summary query loads.
import { useTranslation } from "react-i18next";

import { useAiDecisionsSummary } from "@/ipc/queries/audit";

interface AuditSummaryBarProps {
  /** Scope to one account, or null for all accounts. */
  accountId: string | null;
  sinceUnix: number;
  untilUnix: number;
}

export function AuditSummaryBar({ accountId, sinceUnix, untilUnix }: AuditSummaryBarProps) {
  const { t } = useTranslation("audit");
  const { data: summary, isLoading } = useAiDecisionsSummary(accountId, sinceUnix, untilUnix);

  const blocks: { label: string; value: string | null }[] = [
    {
      label: t("audit_total_events"),
      value: summary ? String(summary.totalEvents) : null,
    },
    {
      label: t("audit_auto_sent"),
      value: summary ? String(summary.autoSentCount) : null,
    },
    {
      label: t("audit_success_rate"),
      value: summary ? `${Math.round(summary.successRate * 100)}%` : null,
    },
    {
      label: t("audit_tokens_used"),
      value: summary
        ? (summary.totalInputTokens + summary.totalOutputTokens).toLocaleString()
        : null,
    },
  ];

  return (
    <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
      {blocks.map((block) => (
        <div
          key={block.label}
          className="rounded-card border border-divider bg-surface p-4 shadow-card"
        >
          {isLoading || block.value === null ? (
            <span
              data-testid="summary-skeleton"
              aria-hidden="true"
              className="block h-7 w-[72px] animate-pulse rounded bg-p5"
            />
          ) : (
            <p className="font-mono text-xl text-p10">{block.value}</p>
          )}
          <p className="section-label mt-1.5">{block.label}</p>
        </div>
      ))}
    </div>
  );
}
