// Channel member list drawer (T093, F_I2 §3). Lists every agent (avatar + name +
// presence) plus a row for the human user (the primary account's owner). Right-
// side overlay; closes on backdrop click or Escape. Presence dot colors come from
// design tokens, never bare hex.

import { useTranslation } from "react-i18next";
import { User } from "lucide-react";
import type { Account } from "@shared/bindings";

import type { AgentStatusValue } from "@/ipc/agents";
import AgentAvatar from "./AgentAvatar";
import AgentNameChip from "./AgentNameChip";

interface MemberDrawerProps {
  open: boolean;
  onClose: () => void;
  accounts: Account[];
  /** account id → presence; missing ids fall back to "idle". */
  statusById: Record<string, AgentStatusValue>;
  /** The primary account, used for the human "You" row identity. */
  primary?: Account;
}

const DOT_VAR: Record<AgentStatusValue, string> = {
  processing: "var(--amber)",
  idle: "var(--green)",
  offline: "var(--p5)",
};

function PresenceDot({ status }: { status: AgentStatusValue }) {
  const { t } = useTranslation("team");
  return (
    <span className="flex shrink-0 items-center gap-1">
      <span
        aria-hidden
        className="inline-block h-2 w-2 rounded-avatar"
        style={{ background: DOT_VAR[status] }}
      />
      <span className="font-ui text-[10px] uppercase tracking-wider text-p8">
        {t(`agent_status_${status}`)}
      </span>
    </span>
  );
}

export default function MemberDrawer({
  open,
  onClose,
  accounts,
  statusById,
  primary,
}: MemberDrawerProps) {
  const { t } = useTranslation("team");
  if (!open) return null;

  return (
    <div
      className="bg-p10/30 fixed inset-0 z-40 flex justify-end"
      onClick={onClose}
      role="presentation"
    >
      <div
        className="flex h-full w-72 flex-col gap-3 bg-surface p-4 shadow-card [border-inline-start:1px_solid_var(--p5)]"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label={t("team_members")}
        onKeyDown={(e) => e.key === "Escape" && onClose()}
      >
        <p className="section-label">{t("team_members")}</p>

        <ul className="flex flex-col gap-2 overflow-y-auto">
          {accounts.map((account) => (
            <li key={account.id} className="flex items-center gap-2">
              <AgentAvatar
                email={account.email}
                colorToken={account.colorToken}
                size={32}
                className="shrink-0"
              />
              <div className="min-w-0 flex-1">
                <AgentNameChip
                  displayName={account.displayName}
                  email={account.email}
                  isPrimary={account.isPrimary}
                />
              </div>
              <PresenceDot status={statusById[account.id] ?? "idle"} />
            </li>
          ))}

          {/* Human user row — the account owner (uses the primary identity). */}
          <li className="mt-1 flex items-center gap-2 border-t border-divider pt-2">
            <span
              aria-hidden
              className="flex h-8 w-8 shrink-0 items-center justify-center rounded-avatar bg-p9 text-white"
            >
              <User size={16} />
            </span>
            <div className="min-w-0 flex-1">
              <p className="truncate font-body text-sm text-p10">{t("team_member_you")}</p>
              <p className="truncate font-mono text-[10px] text-p7">
                {primary?.email ?? t("team_human_role")}
              </p>
            </div>
          </li>
        </ul>
      </div>
    </div>
  );
}
