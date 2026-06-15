// Collapsible right rail (07 §3). Visibility is driven by `ui.agentRailOpen`;
// AppShell mounts it conditionally. v0.1 shows the rail chrome + empty state; the
// agent roster and proactive queries land with the Agent-IM cards (v0.2+).
import { useTranslation } from "react-i18next";

export default function AgentPanel() {
  const { t } = useTranslation("common");
  return (
    <aside className="flex h-full w-72 shrink-0 flex-col gap-4 border-divider bg-parchment p-5 [border-inline-start-width:1px]">
      <p className="section-label">{t("agent_panel_title")}</p>
      <div className="panel-actions flex flex-1 items-start">
        <p className="font-body text-sm text-p7">{t("agent_panel_empty")}</p>
      </div>
    </aside>
  );
}
