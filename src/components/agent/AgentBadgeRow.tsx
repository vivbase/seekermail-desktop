// Dashboard Agent badge row (T102). One status chip per account, primary agent
// first; horizontally scrollable; renders nothing when there are no accounts.
// Presence is polled via useAgentStatuses (30 s) until the event stream lands.
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import { useAccounts } from "@/ipc/queries/accounts";
import { useAgentStatuses } from "@/ipc/queries/agents";
import type { AgentStatusValue } from "@/ipc/agents";
import AgentStatusChip from "./AgentStatusChip";

export default function AgentBadgeRow() {
  const { t } = useTranslation("team");
  const { data: accounts = [] } = useAccounts();
  const { data: statuses = [] } = useAgentStatuses();

  const statusById = useMemo(
    () => Object.fromEntries(statuses.map((s) => [s.accountId, s.status as AgentStatusValue])),
    [statuses],
  );

  // Primary first, then by creation order — mirrors AccountRepo's list ordering.
  const sorted = useMemo(
    () =>
      [...accounts].sort(
        (a, b) => Number(b.isPrimary) - Number(a.isPrimary) || a.createdAt - b.createdAt,
      ),
    [accounts],
  );

  if (accounts.length === 0) return null;

  return (
    <div
      role="group"
      aria-label={t("agent_row_label")}
      className="flex gap-2 overflow-x-auto pb-1 [scrollbar-width:thin]"
    >
      {sorted.map((account) => (
        <AgentStatusChip
          key={account.id}
          account={account}
          status={statusById[account.id] ?? "idle"}
        />
      ))}
    </div>
  );
}
