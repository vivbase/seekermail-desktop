// Profile — the unified settings hub (prototype "Profile" page). A hero (primary
// account avatar + name + tagline + stats) over top tabs that surface the existing
// settings panels: Accounts / AI Model / Privacy & Data / Appearance / About. The
// per-account agent config lives under the Accounts tab (reused AccountsSettings).
// Note: the prototype's 5th tab is "Notifications"; the app ships Appearance
// instead (no notifications-prefs backend yet), so that slot maps to Appearance.
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { useAccounts } from "@/ipc/queries/accounts";
import { useMailCount } from "@/ipc/queries/mail";
import { accountColorClass, type AccountColorToken } from "@/lib/accountColor";
import { cn } from "@/lib/cn";
import PageBack from "@/components/layout/PageBack";
import AccountsSettings from "@/routes/settings/accounts";
import AiProvidersPage from "@/routes/settings/ai";
import PrivacySettings from "@/routes/settings/privacy";
import DataSettings from "@/routes/settings/data";
import AppearanceSettings from "@/routes/settings/appearance";
import AboutSettings from "@/routes/settings/about";

type Tab = "accounts" | "ai" | "privacy" | "appearance" | "about";

const TABS: { id: Tab; labelKey: string }[] = [
  { id: "accounts", labelKey: "profile_tab_accounts" },
  { id: "ai", labelKey: "profile_tab_ai" },
  { id: "privacy", labelKey: "profile_tab_privacy" },
  { id: "appearance", labelKey: "profile_tab_appearance" },
  { id: "about", labelKey: "profile_tab_about" },
];

function Stat({ n, label }: { n: number; label: string }) {
  return (
    <div className="text-center">
      <div className="font-mono text-xl text-p10">{n.toLocaleString()}</div>
      <div className="font-ui text-[9px] uppercase tracking-wider text-p8">{label}</div>
    </div>
  );
}

export default function Profile() {
  const { t } = useTranslation("common");
  const { data: accounts } = useAccounts();
  const { data: indexed } = useMailCount({});
  const [tab, setTab] = useState<Tab>("accounts");

  const list = accounts ?? [];
  const primary = list.find((a) => a.isPrimary) ?? list[0] ?? null;

  return (
    <div className="flex h-full flex-col overflow-hidden">
      {/* Hero */}
      <div className="shrink-0 border-b border-divider px-8 pt-7">
        <PageBack to="/" labelKey="back_to_dashboard" />
        <div className="flex flex-wrap items-center gap-4">
          {primary && (
            <span
              aria-hidden
              className={cn(
                "flex h-14 w-14 shrink-0 items-center justify-center rounded-card font-ui text-lg font-bold",
                accountColorClass(primary.colorToken as AccountColorToken),
              )}
            >
              {primary.badgeLabel}
            </span>
          )}
          <div className="min-w-0 flex-1">
            <h1 className="truncate font-display text-3xl italic text-p10">
              {primary?.displayName ?? t("app_name")}
            </h1>
            <p className="mt-0.5 font-ui text-[11px] uppercase tracking-[0.06em] text-p8">
              {t("app_name")} · {t("profile_tagline")}
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-6">
            <Stat n={list.length} label={t("profile_stat_accounts")} />
            <Stat n={indexed ?? 0} label={t("profile_stat_indexed")} />
          </div>
        </div>

        {/* Top tabs */}
        <div className="mt-5 flex flex-wrap gap-1">
          {TABS.map((tb) => (
            <button
              key={tb.id}
              type="button"
              onClick={() => setTab(tb.id)}
              aria-current={tab === tb.id}
              className={cn(
                "border-b-2 px-3 py-2 font-ui text-xs uppercase tracking-wider transition-colors",
                tab === tb.id ? "border-p10 text-p10" : "border-transparent text-p8 hover:text-p10",
              )}
            >
              {t(tb.labelKey)}
            </button>
          ))}
        </div>
      </div>

      {/* Tab content — reuses the existing settings panels */}
      <div className="min-h-0 flex-1 overflow-y-auto px-8 py-6">
        {tab === "accounts" && <AccountsSettings />}
        {tab === "ai" && <AiProvidersPage />}
        {tab === "privacy" && (
          <div className="space-y-8">
            <PrivacySettings />
            <DataSettings />
          </div>
        )}
        {tab === "appearance" && <AppearanceSettings />}
        {tab === "about" && <AboutSettings />}
      </div>
    </div>
  );
}
