// The always-mounted shell, rebuilt to match the prototype: a left Sidebar and the
// routed main area only — no global command bar, and no right agent rail (Agent-IM
// lives on the Team page). The T4 risk banner stays pinned above everything; the
// Cmd/Ctrl+K shortcut is registered here and navigates to the /search page so search
// is reachable from anywhere (Search is now a first-class route, not an overlay).
import { Suspense, useEffect } from "react";
import { Navigate, Outlet, useLocation, useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";

import Sidebar from "./Sidebar";
import RiskBanner from "./RiskBanner";
import AiActivationPrompt from "@/components/ai/AiActivationPrompt";
import { ToastViewport } from "@/components/ui/Toast";
import { useUi } from "@/stores/ui";
import { useHasAccounts } from "@/lib/accountGate";
import { useFontScaleShortcuts } from "@/hooks/useFontScaleShortcuts";

function RouteFallback() {
  const { t } = useTranslation("common");
  return (
    <div className="flex h-full items-center justify-center">
      <p className="font-body text-p7">{t("state_loading")}</p>
    </div>
  );
}

export default function AppShell() {
  const location = useLocation();
  const navigate = useNavigate();
  const setActiveRoute = useUi((s) => s.setActiveRoute);
  const hasAccounts = useHasAccounts();

  // Mirror the router path into the store to drive sidebar highlight (07 §4).
  useEffect(() => {
    setActiveRoute(location.pathname);
  }, [location.pathname, setActiveRoute]);

  // Global Cmd+K (macOS) / Ctrl+K — jump to the /search page from anywhere (Search
  // is a first-class route, not an overlay). Registered once on mount.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && (e.key === "k" || e.key === "K")) {
        e.preventDefault();
        navigate("/search");
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [navigate]);

  // Global UI-scale shortcuts: Cmd/Ctrl + / - / 0 (analysis 25 follow-up).
  useFontScaleShortcuts();

  // The only routing gate: no accounts → onboarding.
  if (!hasAccounts) {
    return <Navigate to="/onboarding" replace />;
  }

  return (
    <div className="flex h-full w-full flex-col overflow-hidden bg-parchment text-p10">
      {/* T100: app-wide T4 risk banner — above all routed content, non-dismissable. */}
      <RiskBanner />
      <div className="app flex min-h-0 w-full flex-1 overflow-hidden">
        <Sidebar />
        <main className="min-h-0 flex-1 overflow-auto bg-parchment">
          <Suspense fallback={<RouteFallback />}>
            <Outlet />
          </Suspense>
        </main>
        {/* Toast queue (T078/T081) — survives route changes. */}
        <ToastViewport />
      </div>
      {/* First-run AI activation nudge — dismissible, optional (not a gate). */}
      <AiActivationPrompt />
    </div>
  );
}
