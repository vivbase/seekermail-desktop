// Per-tab inner shell (Model S, WB-09 v2): the Sidebar + routed main area for ONE workbench
// tab. Global chrome (T4 risk banner, toasts, prompts, account gate, app-wide shortcuts) lives
// in WorkbenchShell — NOT here — so it renders once, not per tab. Each tab renders this inside
// its OWN MemoryRouter, giving fully independent navigation per tab.
import { Suspense } from "react";
import { Outlet } from "react-router-dom";
import { useTranslation } from "react-i18next";

import Sidebar from "./Sidebar";

function RouteFallback() {
  const { t } = useTranslation("common");
  return (
    <div className="flex h-full items-center justify-center">
      <p className="font-body text-p7">{t("state_loading")}</p>
    </div>
  );
}

export default function WorkspaceLayout() {
  return (
    <div className="app flex min-h-0 w-full flex-1 overflow-hidden">
      <Sidebar />
      <main className="min-h-0 flex-1 overflow-auto bg-parchment">
        <Suspense fallback={<RouteFallback />}>
          <Outlet />
        </Suspense>
      </main>
    </div>
  );
}
