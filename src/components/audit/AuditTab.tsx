// "AI Activity" tab container (T089 §3): kill-switch banner + summary bar +
// filters + export + decision list + detail drawer. Filter state is plain
// useState (no persistence); the drawer entry is local selection state.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { AiDecisionRow } from "@shared/bindings";

import { EMPTY_AUDIT_FILTERS, type AuditListFilters } from "@/ipc/queries/audit";
import { useE3PausedUntil, useSetE3PausedUntil } from "@/ipc/queries/settings";

import { AuditDrawer } from "./AuditDrawer";
import { AuditFilters } from "./AuditFilters";
import { AuditList } from "./AuditList";
import { AuditSummaryBar } from "./AuditSummaryBar";
import { ExportButton } from "./ExportButton";

/** Default summary/export window when no date filter is set (F_E7 §4.6). */
const DEFAULT_WINDOW_DAYS = 30;

export function AuditTab() {
  const { t } = useTranslation("audit");
  const [filters, setFilters] = useState<AuditListFilters>({ ...EMPTY_AUDIT_FILTERS });
  const [drawerEntry, setDrawerEntry] = useState<AiDecisionRow | null>(null);

  const { data: pausedUntil = 0 } = useE3PausedUntil();
  const setPausedUntil = useSetE3PausedUntil();

  const now = Math.floor(Date.now() / 1000);
  const sinceUnix = filters.sinceUnix ?? now - DEFAULT_WINDOW_DAYS * 86_400;
  const untilUnix = filters.untilUnix ?? now;
  const summaryAccountId = filters.accountIds.length === 1 ? filters.accountIds[0]! : null;

  const paused = pausedUntil > now;

  return (
    <div className="space-y-4">
      {/* Kill-switch banner (T086/T089): visible while Full Auto is paused. */}
      {paused && (
        <div
          role="status"
          className="bg-amber/10 flex flex-wrap items-center justify-between gap-2 rounded-card border border-amber px-4 py-2.5"
        >
          <p className="font-body text-sm text-p9">
            {t("audit_e3_paused_banner", {
              date: new Date(pausedUntil * 1000).toLocaleString(),
            })}
          </p>
          <button
            type="button"
            disabled={setPausedUntil.isPending}
            onClick={() => setPausedUntil.mutate(0)}
            className="hover:bg-amber/15 shrink-0 rounded-chip px-2.5 py-1 font-ui text-[10px] font-semibold uppercase tracking-wider text-amber transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-p9 disabled:opacity-50"
          >
            {t("audit_e3_resume_early")}
          </button>
        </div>
      )}

      <AuditSummaryBar accountId={summaryAccountId} sinceUnix={sinceUnix} untilUnix={untilUnix} />

      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <AuditFilters value={filters} onChange={setFilters} />
        </div>
        <ExportButton filters={filters} defaultSinceUnix={sinceUnix} defaultUntilUnix={untilUnix} />
      </div>

      <AuditList filters={filters} onSelect={setDrawerEntry} selectedId={drawerEntry?.id ?? null} />

      <AuditDrawer entry={drawerEntry} onClose={() => setDrawerEntry(null)} />
    </div>
  );
}
