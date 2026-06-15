// TEAM channel top bar (T093, F_I2 §4). Rebuilt to match the prototype: title
// "Team" + "N AI Employees · M pending", a "+ New Query" button (focuses the
// composer), the disabled search affordance, and an overlapping member-avatar
// group (with presence pips) that toggles the member drawer.

import { Search } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { Account } from "@shared/bindings";

import type { AgentStatusValue } from "@/ipc/agents";
import { cn } from "@/lib/cn";
import AgentAvatar from "./AgentAvatar";

interface ChannelTopbarProps {
  accounts: Account[];
  statusById: Record<string, AgentStatusValue>;
  agentCount: number;
  pendingCount: number;
  membersOpen: boolean;
  onToggleMembers: () => void;
  onNewQuery: () => void;
}

/** Presence pip tone for a member avatar. */
function pipClass(status: AgentStatusValue | undefined): string {
  if (status === "processing") return "bg-amber";
  if (status === "offline") return "bg-p6";
  return "bg-green"; // idle / online / unknown
}

export default function ChannelTopbar({
  accounts,
  statusById,
  agentCount,
  pendingCount,
  membersOpen,
  onToggleMembers,
  onNewQuery,
}: ChannelTopbarProps) {
  const { t } = useTranslation("team");

  return (
    <header className="flex items-center justify-between gap-3 border-b border-divider bg-surface px-5 py-3">
      <div className="min-w-0">
        <h1 className="font-display text-lg italic text-p10">{t("team_title")}</h1>
        <p className="font-ui text-[11px] uppercase tracking-wider text-p8">
          {t("team_employees_count", { count: agentCount })}
          {pendingCount > 0 && <> · {t("team_pending_short", { count: pendingCount })}</>}
        </p>
      </div>

      <div className="flex shrink-0 items-center gap-2">
        <button
          type="button"
          onClick={onNewQuery}
          className="rounded-chip border border-divider px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p9 transition-colors hover:bg-p4"
        >
          {t("team_new_query")}
        </button>

        <button
          type="button"
          disabled
          title={t("team_search_soon")}
          aria-label={t("team_search_soon")}
          className="rounded-chip p-2 text-p7 disabled:cursor-not-allowed disabled:opacity-50"
        >
          <Search size={16} aria-hidden />
        </button>

        {/* Member-avatar group → toggles the member drawer */}
        <button
          type="button"
          onClick={onToggleMembers}
          aria-expanded={membersOpen}
          aria-label={membersOpen ? t("team_members_close") : t("team_members_open")}
          className="flex items-center gap-2 rounded-chip border border-divider px-2.5 py-1 transition-colors hover:bg-p4"
        >
          {accounts.length > 0 && (
            <span className="flex -space-x-2">
              {accounts.slice(0, 4).map((a) => (
                <span key={a.id} className="relative">
                  <AgentAvatar
                    email={a.email}
                    colorToken={a.colorToken}
                    size={24}
                    className="ring-1 ring-surface"
                  />
                  <span
                    aria-hidden
                    className={cn(
                      "absolute -bottom-0.5 -end-0.5 h-2 w-2 rounded-avatar ring-1 ring-surface",
                      pipClass(statusById[a.id]),
                    )}
                  />
                </span>
              ))}
            </span>
          )}
          <span className="font-ui text-xs uppercase tracking-wider text-p9">{t("team_members")}</span>
        </button>
      </div>
    </header>
  );
}
