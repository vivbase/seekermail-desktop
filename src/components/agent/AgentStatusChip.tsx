// Compact Agent status chip for the Dashboard (T102, F_I2 §3.2/§4.2). Avatar +
// name (+ ★ for the primary agent) + a presence dot; clicking jumps to the TEAM
// channel. Presence colors are design tokens; the "processing" dot spins via the
// shared keyframe in tokens.css. Rendered as a <button> for keyboard access.
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import type { Account } from "@shared/bindings";

import type { AgentStatusValue } from "@/ipc/agents";
import AgentAvatar from "./AgentAvatar";
import AgentNameChip from "./AgentNameChip";

interface AgentStatusChipProps {
  account: Account;
  status: AgentStatusValue;
}

const DOT_VAR: Record<AgentStatusValue, string> = {
  processing: "var(--amber)",
  idle: "var(--green)",
  offline: "var(--p5)",
};

export default function AgentStatusChip({ account, status }: AgentStatusChipProps) {
  const { t } = useTranslation("team");
  const navigate = useNavigate();
  const statusLabel = t(`agent_status_${status}`);

  return (
    <button
      type="button"
      onClick={() => navigate("/team")}
      aria-label={t("agent_chip_aria", { name: account.displayName, status: statusLabel })}
      title={`${account.email} · ${statusLabel}`}
      className="flex shrink-0 items-center gap-1.5 rounded-chip bg-p3 px-2 py-1 transition-colors hover:bg-p5"
    >
      <AgentAvatar email={account.email} colorToken={account.colorToken} size={24} />
      <AgentNameChip
        displayName={account.displayName}
        email={account.email}
        isPrimary={account.isPrimary}
        hideDomain
      />
      <span
        aria-hidden
        className={status === "processing" ? "agent-status-dot--spinning" : undefined}
        style={{
          width: 8,
          height: 8,
          borderRadius: "var(--radius-avatar)",
          background: DOT_VAR[status],
        }}
      />
    </button>
  );
}
