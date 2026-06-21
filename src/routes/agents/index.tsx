// Agents page (T073) — rebuilt to match the prototype's roster dashboard: a
// Global-Mode aggregate indicator, summary tiles (active / idle / paused /
// queries), filter chips + search, and one row per account showing the agent's
// avatar, auth level, presence, and pending-query count. Each row's "View"
// expands the existing per-account AgentCard config editor in place (config is
// unchanged; only the surrounding roster is new). Live status comes from
// useAgentStatuses; per-agent query counts are grouped from the pending-queries
// list. Processed totals and the trust-ramp meter are intentionally omitted here
// until the backend exposes them (no fabricated metrics).
import { useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { useAccounts } from "@/ipc/queries/accounts";
import { useAgentStatuses } from "@/ipc/queries/agents";
import { usePendingQueries } from "@/ipc/queries/queries";
import type { AgentStatusValue } from "@/ipc/agents";
import { accountColorClass, type AccountColorToken } from "@/lib/accountColor";
import { cn } from "@/lib/cn";
import AgentCard from "./AgentCard";

type Filter = "all" | "active" | "idle" | "paused";
type Bucket = "active" | "idle" | "paused";

const AUTH_LABEL: Record<number, string> = {
  1: "agents_auth_manual",
  2: "agents_auth_semi",
  3: "agents_auth_full_auto",
};

/** Map presence + enabled flag to a roster bucket. */
function statusBucket(status: AgentStatusValue | undefined, isActive: boolean): Bucket {
  if (!isActive || status === "offline") return "paused";
  if (status === "processing") return "active";
  return "idle";
}

const DOT: Record<Bucket, string> = {
  active: "var(--amber)",
  idle: "var(--green)",
  paused: "var(--p5)",
};

export default function Agents() {
  const { t } = useTranslation(["agents", "nav", "common"]);
  const { data: accounts, isLoading } = useAccounts();
  const { data: statuses = [] } = useAgentStatuses();
  const { data: queries = [] } = usePendingQueries(null);

  const [filter, setFilter] = useState<Filter>("all");
  const [search, setSearch] = useState("");
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const toastTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const showToast = (m: string) => {
    if (toastTimer.current) clearTimeout(toastTimer.current);
    setToast(m);
    toastTimer.current = setTimeout(() => setToast(null), 2800);
  };

  const statusById = useMemo(
    () => Object.fromEntries(statuses.map((s) => [s.accountId, s.status as AgentStatusValue])),
    [statuses],
  );
  const queryCountById = useMemo(() => {
    const m: Record<string, number> = {};
    for (const q of queries) m[q.accountId] = (m[q.accountId] ?? 0) + 1;
    return m;
  }, [queries]);

  const sorted = useMemo(
    () => [...(accounts ?? [])].sort((a, b) => a.createdAt - b.createdAt),
    [accounts],
  );

  const buckets = useMemo(() => {
    const b: Record<Bucket, number> = { active: 0, idle: 0, paused: 0 };
    for (const a of sorted) b[statusBucket(statusById[a.id], a.isActive)]++;
    return b;
  }, [sorted, statusById]);
  const queriesPending = queries.length;

  // Aggregate auth mode across accounts (0 = mixed).
  const authLevels = new Set(sorted.map((a) => a.authLevel));
  const aggMode = authLevels.size === 1 ? [...authLevels][0]! : 0;

  const visible = sorted.filter((a) => {
    if (filter !== "all" && statusBucket(statusById[a.id], a.isActive) !== filter) return false;
    if (
      search.trim() &&
      !`${a.displayName} ${a.email}`.toLowerCase().includes(search.toLowerCase())
    )
      return false;
    return true;
  });

  const tiles: { key: Filter | "queries"; color: string; n: number; lbl: string }[] = [
    { key: "active", color: "var(--green)", n: buckets.active, lbl: "agents_stat_active" },
    { key: "idle", color: "", n: buckets.idle, lbl: "agents_stat_idle" },
    { key: "paused", color: "var(--amber)", n: buckets.paused, lbl: "agents_stat_paused" },
    { key: "queries", color: "var(--terra)", n: queriesPending, lbl: "agents_stat_queries" },
  ];

  return (
    <section className="flex h-full flex-col overflow-y-auto">
      {/* Header: title + roster subtitle + Global-Mode aggregate indicator */}
      <header className="shrink-0 border-b border-divider px-6 py-5">
        <div className="flex flex-wrap items-end justify-between gap-3">
          <div>
            <h1 className="font-display text-3xl italic text-p10">
              {t("agents:agents_page_title")}
            </h1>
            <p className="mt-1 font-ui text-[11px] uppercase tracking-[0.06em] text-p8">
              {t("agents:agents_roster_subtitle", {
                accounts: sorted.length,
                queries: queriesPending,
                active: buckets.active,
              })}
            </p>
          </div>
          <div className="flex items-center gap-2 rounded-chip border border-divider bg-surface px-3 py-1.5">
            <span className="font-ui text-[9px] uppercase tracking-wider text-p8">
              {t("agents:agents_global_mode")}
            </span>
            <span className="rounded-chip bg-p4 px-2 py-0.5 font-ui text-[10px] uppercase tracking-wider text-p9">
              {aggMode === 0 ? t("agents:agents_mode_mixed") : t(`agents:${AUTH_LABEL[aggMode]}`)}
            </span>
          </div>
        </div>
      </header>

      <div className="px-6 py-5">
        {/* Summary tiles (click to filter) */}
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
          {tiles.map((tile) => (
            <button
              key={tile.key}
              type="button"
              onClick={() => tile.key !== "queries" && setFilter(tile.key)}
              className="rounded-card border border-divider bg-surface p-4 text-start shadow-card transition-colors hover:bg-p2"
            >
              <div
                className="font-mono text-2xl"
                style={tile.color ? { color: tile.color } : undefined}
              >
                {tile.n}
              </div>
              <div className="mt-1 font-ui text-[10px] uppercase tracking-wider text-p8">
                {t(`agents:${tile.lbl}`)}
              </div>
            </button>
          ))}
        </div>

        {/* Filter chips + search */}
        <div className="mt-4 flex flex-wrap items-center gap-2">
          {(["all", "active", "idle", "paused"] as const).map((f) => (
            <button
              key={f}
              type="button"
              onClick={() => setFilter(f)}
              className={cn(
                "rounded-chip px-3 py-1.5 font-ui text-xs uppercase tracking-wider transition-colors",
                filter === f ? "bg-p9 text-white" : "text-p8 hover:bg-p4",
              )}
            >
              {t(`agents:agents_filter_${f}`)} {f === "all" ? sorted.length : buckets[f]}
            </button>
          ))}
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            aria-label={t("agents:agents_search_ph")}
            placeholder={t("agents:agents_search_ph")}
            className="ms-auto rounded-chip border border-divider bg-surface px-3 py-1.5 font-body text-sm text-p10 placeholder:text-p7 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
          />
        </div>

        {/* Agent rows */}
        <div className="mt-4 space-y-2">
          {isLoading && <p className="font-body text-p7">{t("common:state_loading")}</p>}
          {!isLoading && sorted.length === 0 && (
            <p className="font-body text-p7">{t("agents:agents_empty")}</p>
          )}
          {visible.map((account) => {
            const bucket = statusBucket(statusById[account.id], account.isActive);
            const qc = queryCountById[account.id] ?? 0;
            const expanded = expandedId === account.id;
            return (
              <div
                key={account.id}
                className="rounded-card border border-divider bg-surface shadow-card"
              >
                <div className="flex flex-wrap items-center gap-3 p-4">
                  <span
                    aria-hidden
                    className={cn(
                      "flex h-9 w-9 shrink-0 items-center justify-center rounded-avatar font-ui text-sm",
                      accountColorClass(account.colorToken as AccountColorToken),
                    )}
                  >
                    {account.badgeLabel}
                  </span>
                  <div className="min-w-0 flex-1">
                    <span className="flex items-center gap-1.5">
                      <span className="truncate font-body text-sm text-p10">
                        {account.displayName}
                      </span>
                      {account.isPrimary && (
                        <span className="text-amber" title={t("agents:primary_account_badge")}>
                          ★
                        </span>
                      )}
                    </span>
                    <span className="block truncate font-mono text-xs text-p8">
                      {account.email}
                    </span>
                  </div>
                  <span className="shrink-0 rounded-chip bg-p4 px-2 py-0.5 font-ui text-[9px] uppercase tracking-wider text-p9">
                    {t(`agents:${AUTH_LABEL[account.authLevel] ?? "agents_auth_manual"}`)}
                  </span>
                  <span className="flex shrink-0 items-center gap-1.5">
                    <span
                      aria-hidden
                      className="h-2 w-2 rounded-avatar"
                      style={{ background: DOT[bucket] }}
                    />
                    <span className="font-ui text-[10px] uppercase tracking-wider text-p8">
                      {t(`agents:agents_status_${bucket}`)}
                    </span>
                  </span>
                  {qc > 0 && (
                    <span className="shrink-0 rounded-chip bg-terra px-2 py-0.5 font-ui text-[9px] uppercase tracking-wider text-white">
                      {t("agents:agents_query_count", { count: qc })}
                    </span>
                  )}
                  <button
                    type="button"
                    onClick={() => setExpandedId(expanded ? null : account.id)}
                    aria-expanded={expanded}
                    className="shrink-0 rounded-chip border border-divider px-3 py-1.5 font-ui text-[10px] uppercase tracking-wider text-p9 transition-colors hover:bg-p4"
                  >
                    {expanded ? t("agents:agents_collapse") : t("agents:agents_view")}
                  </button>
                </div>
                {expanded && (
                  <div className="border-t border-divider p-4">
                    <AgentCard account={account} onSaved={showToast} />
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>

      {toast && (
        <div
          role="status"
          aria-live="polite"
          className="fixed bottom-6 z-50 rounded-card bg-p9 px-4 py-3 font-ui text-sm text-surface shadow-card [inset-inline-end:1.5rem]"
        >
          {toast}
        </div>
      )}
    </section>
  );
}
