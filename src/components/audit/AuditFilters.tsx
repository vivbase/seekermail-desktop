// E7 list filters (T089 §3, F_E7 §4.5): account multi-select (color chips),
// event-type multi-select, and a from/to date range. Pure useState lifted to
// AuditTab — no persistence (no localStorage by hard rule).
import { useTranslation } from "react-i18next";
import { DECISION_TYPES, type Account } from "@shared/bindings";

import { useAccounts } from "@/ipc/queries/accounts";
import { EMPTY_AUDIT_FILTERS, type AuditListFilters } from "@/ipc/queries/audit";
import { accountColorClass, type AccountColorToken } from "@/lib/accountColor";
import { cn } from "@/lib/cn";

import { eventTypeLabel } from "./eventColor";

interface AuditFiltersProps {
  value: AuditListFilters;
  onChange: (next: AuditListFilters) => void;
}

const DAY_SECS = 86_400;

/** unix → yyyy-mm-dd for <input type="date"> (local timezone). */
function toDateInput(unix: number | null): string {
  if (unix === null) return "";
  const d = new Date(unix * 1000);
  const month = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${d.getFullYear()}-${month}-${day}`;
}

/** yyyy-mm-dd → unix at local midnight; null for empty input. */
function fromDateInput(value: string): number | null {
  if (value === "") return null;
  const parsed = new Date(`${value}T00:00:00`);
  const unix = Math.floor(parsed.getTime() / 1000);
  return Number.isFinite(unix) ? unix : null;
}

export function AuditFilters({ value, onChange }: AuditFiltersProps) {
  const { t } = useTranslation("audit");
  const { data: accounts } = useAccounts();

  function toggleAccount(id: string) {
    const next = value.accountIds.includes(id)
      ? value.accountIds.filter((a) => a !== id)
      : [...value.accountIds, id];
    onChange({ ...value, accountIds: next });
  }

  function toggleEventType(type: string) {
    const next = value.eventTypes.includes(type)
      ? value.eventTypes.filter((e) => e !== type)
      : [...value.eventTypes, type];
    onChange({ ...value, eventTypes: next });
  }

  const hasActiveFilters =
    value.accountIds.length > 0 ||
    value.eventTypes.length > 0 ||
    value.sinceUnix !== null ||
    value.untilUnix !== null;

  return (
    <div className="rounded-card border border-divider bg-surface p-4 shadow-card">
      <div className="flex flex-wrap items-start gap-x-6 gap-y-3">
        {/* Account multi-select */}
        <fieldset className="min-w-0">
          <legend className="section-label mb-1.5">{t("audit_filter_account")}</legend>
          <div className="flex flex-wrap items-center gap-1.5">
            {(accounts ?? []).map((account: Account) => {
              const selected = value.accountIds.includes(account.id);
              return (
                <button
                  key={account.id}
                  type="button"
                  aria-pressed={selected}
                  onClick={() => toggleAccount(account.id)}
                  className={cn(
                    "flex items-center gap-1.5 rounded-chip border px-2.5 py-1 font-ui text-[10px] font-semibold uppercase tracking-wider transition-colors",
                    "focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
                    selected ? "border-p9 bg-p4 text-p10" : "border-divider text-p8 hover:bg-p4",
                  )}
                >
                  <span
                    aria-hidden="true"
                    className={cn(
                      "flex h-4 w-4 items-center justify-center rounded-avatar text-[8px] font-semibold",
                      accountColorClass((account.colorToken as AccountColorToken) ?? "team"),
                    )}
                  >
                    {account.badgeLabel}
                  </span>
                  {account.displayName}
                </button>
              );
            })}
          </div>
        </fieldset>

        {/* Event-type multi-select */}
        <fieldset className="min-w-0">
          <legend className="section-label mb-1.5">{t("audit_filter_event_type")}</legend>
          <div className="flex max-w-md flex-wrap items-center gap-1.5">
            {DECISION_TYPES.map((type) => {
              const selected = value.eventTypes.includes(type);
              return (
                <button
                  key={type}
                  type="button"
                  aria-pressed={selected}
                  onClick={() => toggleEventType(type)}
                  className={cn(
                    "rounded-chip border px-2 py-1 font-ui text-[9px] font-semibold uppercase tracking-wider transition-colors",
                    "focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
                    selected ? "border-p9 bg-p4 text-p10" : "border-divider text-p8 hover:bg-p4",
                  )}
                >
                  {eventTypeLabel(type)}
                </button>
              );
            })}
          </div>
        </fieldset>

        {/* Date range */}
        <fieldset>
          <legend className="section-label mb-1.5">{t("audit_filter_date_range")}</legend>
          <div className="flex items-center gap-2">
            <label className="flex items-center gap-1.5 font-ui text-[10px] uppercase tracking-wider text-p8">
              {t("audit_filter_from")}
              <input
                type="date"
                value={toDateInput(value.sinceUnix)}
                onChange={(e) => onChange({ ...value, sinceUnix: fromDateInput(e.target.value) })}
                className="rounded-chip border border-divider bg-p1 px-2 py-1 font-mono text-xs text-p9 focus:outline-none focus:ring-1 focus:ring-p9"
              />
            </label>
            <label className="flex items-center gap-1.5 font-ui text-[10px] uppercase tracking-wider text-p8">
              {t("audit_filter_to")}
              <input
                type="date"
                value={toDateInput(
                  value.untilUnix === null ? null : value.untilUnix - (DAY_SECS - 1),
                )}
                onChange={(e) => {
                  const start = fromDateInput(e.target.value);
                  // Make the "to" day inclusive: store end-of-day.
                  onChange({ ...value, untilUnix: start === null ? null : start + DAY_SECS - 1 });
                }}
                className="rounded-chip border border-divider bg-p1 px-2 py-1 font-mono text-xs text-p9 focus:outline-none focus:ring-1 focus:ring-p9"
              />
            </label>
          </div>
        </fieldset>
      </div>

      {hasActiveFilters && (
        <button
          type="button"
          onClick={() => onChange({ ...EMPTY_AUDIT_FILTERS })}
          className="mt-3 font-ui text-[10px] font-semibold uppercase tracking-wider text-p9 underline transition-colors hover:text-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
        >
          {t("audit_clear_filters")}
        </button>
      )}
    </div>
  );
}
