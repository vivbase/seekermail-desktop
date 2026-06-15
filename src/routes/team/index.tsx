// TEAM channel route (T093). Renders the shared Agent-IM channel, or an empty
// state when no accounts exist yet (no agents to populate the channel).
import { useTranslation } from "react-i18next";

import { useAccounts } from "@/ipc/queries/accounts";
import TeamChannel from "@/components/agent/TeamChannel";

export default function Team() {
  const { t } = useTranslation("team");
  const { data: accounts, isLoading } = useAccounts();

  if (!isLoading && (accounts?.length ?? 0) === 0) {
    return (
      <section className="flex h-full items-center justify-center p-8">
        <div className="max-w-md rounded-card border border-divider bg-surface p-6 text-center shadow-card">
          <h1 className="font-display text-2xl italic text-p10">{t("team_channel_name")}</h1>
          <p className="mt-2 font-body text-sm text-p7">{t("team_empty_state")}</p>
        </div>
      </section>
    );
  }

  return <TeamChannel />;
}
