// E7 decision list (T089 §3, F_E7 §4.4). Plain 200-row table (no infinite
// scroll in v0.7); a hint row appears at the cap. `isFetching` drives a thin
// green progress bar so refetches never blank the table.
import { useTranslation } from "react-i18next";
import type { AiDecisionRow } from "@shared/bindings";

import { useAccounts } from "@/ipc/queries/accounts";
import { AUDIT_LIST_LIMIT, useAiDecisions, type AuditListFilters } from "@/ipc/queries/audit";
import { cn } from "@/lib/cn";

import { eventColorVar, eventTypeLabel } from "./eventColor";

const SUBJECT_MAX_CHARS = 40;

/** "Jun 2 · 14:03" — date plus time-of-day (F_E7 §4.4). */
function formatEventTime(unixSeconds: number): string {
  const time = new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(unixSeconds * 1000));
  const day = new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
  }).format(new Date(unixSeconds * 1000));
  return `${day} · ${time}`;
}

function truncate(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max)}…` : text;
}

function tokensLabel(row: AiDecisionRow): string {
  const total = (row.inputTokens ?? 0) + (row.outputTokens ?? 0);
  return total > 0 ? total.toLocaleString() : "—";
}

interface AuditListProps {
  filters: AuditListFilters;
  onSelect: (row: AiDecisionRow) => void;
  selectedId: string | null;
}

export function AuditList({ filters, onSelect, selectedId }: AuditListProps) {
  const { t } = useTranslation("audit");
  const { data: rows, isLoading, isFetching } = useAiDecisions(filters);
  const { data: accounts } = useAccounts();

  const accountName = (accountId: string) =>
    accounts?.find((a) => a.id === accountId)?.displayName ?? accountId;

  return (
    <div className="overflow-hidden rounded-card border border-divider bg-surface shadow-card">
      {/* Refetch indicator: thin animated bar, table stays in place. */}
      <div aria-hidden="true" className="h-px">
        {isFetching && !isLoading && <div className="h-px w-full animate-pulse bg-green" />}
      </div>

      {isLoading ? (
        <p className="p-5 font-body text-sm text-p7">{t("audit_loading")}</p>
      ) : (rows ?? []).length === 0 ? (
        <p className="p-5 font-body text-sm text-p7">{t("audit_empty")}</p>
      ) : (
        <table className="w-full border-collapse">
          <thead>
            <tr className="border-b border-divider">
              <th scope="col" className="section-label px-3 py-2 text-start">
                {t("audit_col_id")}
              </th>
              <th scope="col" className="section-label px-3 py-2 text-start">
                {t("audit_col_account")}
              </th>
              <th scope="col" className="section-label px-3 py-2 text-start">
                {t("audit_col_event")}
              </th>
              <th scope="col" className="section-label px-3 py-2 text-start">
                {t("audit_col_subject")}
              </th>
              <th scope="col" className="section-label px-3 py-2 text-start">
                {t("audit_col_time")}
              </th>
              <th scope="col" className="section-label px-3 py-2 text-end">
                {t("audit_col_tokens")}
              </th>
            </tr>
          </thead>
          <tbody>
            {(rows ?? []).map((row) => (
              <tr
                key={row.id}
                onClick={() => onSelect(row)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    onSelect(row);
                  }
                }}
                tabIndex={0}
                aria-selected={row.id === selectedId}
                className={cn(
                  "cursor-pointer border-b border-divider transition-colors last:border-b-0",
                  "hover:bg-p3 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
                  row.id === selectedId && "bg-p3",
                )}
              >
                <td className="px-3 py-2 font-mono text-[10px] text-p7">{row.id.slice(-6)}</td>
                <td className="max-w-[10rem] truncate px-3 py-2 font-ui text-xs text-p9">
                  {accountName(row.accountId)}
                </td>
                <td className="px-3 py-2">
                  <span
                    className="rounded-chip px-2 py-0.5 font-ui text-[9px] font-semibold uppercase tracking-widest text-white"
                    style={{ background: eventColorVar(row.decisionType) }}
                  >
                    {eventTypeLabel(row.decisionType)}
                  </span>
                </td>
                <td className="max-w-[16rem] truncate px-3 py-2 font-body text-xs text-p9">
                  {row.mailSubject ? truncate(row.mailSubject, SUBJECT_MAX_CHARS) : "—"}
                </td>
                <td className="whitespace-nowrap px-3 py-2 font-mono text-[10px] text-p7">
                  {formatEventTime(row.createdAt)}
                </td>
                <td className="px-3 py-2 text-end font-mono text-[10px] text-p7">
                  {tokensLabel(row)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {(rows ?? []).length >= AUDIT_LIST_LIMIT && (
        <p className="border-t border-divider px-3 py-2 font-body text-xs italic text-p7">
          {t("audit_showing_limit_hint")}
        </p>
      )}
    </div>
  );
}
