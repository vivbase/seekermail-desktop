// Settings layout shell (T049). Wraps all settings sub-routes: SettingsNav on the
// left, <Outlet/> on the right. Sits inside AppShell (inherits the main area).
// /settings itself redirects to /settings/accounts (see App.tsx nested routes).
import { Navigate, Outlet } from "react-router-dom";
import { useTranslation } from "react-i18next";

import PageBack from "@/components/layout/PageBack";
import { SettingsNav } from "./SettingsNav";

export default function SettingsShell() {
  const { t } = useTranslation("settings");

  return (
    <div className="flex h-full overflow-hidden bg-surface">
      {/* Left category navigation */}
      <SettingsNav />

      {/* Right content panel */}
      <div className="flex min-w-0 flex-1 flex-col overflow-y-auto">
        {/* Settings page header. The shared back affordance returns to the
            Dashboard from any settings sub-route (including data/* and ai/*
            drill-downs); lateral movement stays in the left SettingsNav. */}
        <header className="shrink-0 border-b border-divider px-8 py-5">
          <PageBack to="/" labelKey="back_to_dashboard" />
          <h1 className="font-display text-3xl italic text-p10">{t("title")}</h1>
        </header>

        {/* Sub-route content */}
        <main className="flex-1 px-8 py-6">
          <Outlet />
        </main>
      </div>
    </div>
  );
}

// Named re-export so the integrator can also import just the redirect helper.
export function SettingsIndexRedirect() {
  return <Navigate to="/settings/accounts" replace />;
}
