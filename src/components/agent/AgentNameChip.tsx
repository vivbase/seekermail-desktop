// Agent name + identity chip (T094, F_I2 §3.2). Shows the agent's display name,
// a gold ★ for the primary agent, and a small mailbox-domain label; hovering the
// chip reveals the full email address (native title tooltip). Copy is English-only
// and design-token styled.

import { useTranslation } from "react-i18next";

import { cn } from "@/lib/cn";

/** Decorative gold star glyph for the primary agent (not translatable copy). */
const PRIMARY_STAR = "★";

interface AgentNameChipProps {
  displayName: string;
  email: string;
  isPrimary?: boolean;
  /** Hide the small domain label (compact contexts like the dashboard chip). */
  hideDomain?: boolean;
  className?: string;
}

export default function AgentNameChip({
  displayName,
  email,
  isPrimary = false,
  hideDomain = false,
  className,
}: AgentNameChipProps) {
  const { t } = useTranslation("team");
  const domain = email.split("@")[1] ?? "";

  return (
    <span className={cn("inline-flex min-w-0 items-center gap-1", className)} title={email}>
      <span className="truncate font-body text-sm text-p10">{displayName}</span>
      {isPrimary && (
        <span aria-label={t("agent_primary_aria")} className="shrink-0 font-ui text-amber">
          {PRIMARY_STAR}
        </span>
      )}
      {!hideDomain && domain && (
        <span
          aria-label={t("agent_domain_aria", { domain })}
          className="shrink-0 font-mono text-[10px] text-p7"
        >
          @{domain}
        </span>
      )}
    </span>
  );
}
