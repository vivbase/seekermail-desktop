// Live workbench shell (WB-09 v2, Model S). Global chrome renders ONCE on top: the T4 risk
// banner, the tab strip, the quick-switcher, app shortcuts. Below it, every open tab is a
// FULLY INDEPENDENT app instance — its own MemoryRouter + Sidebar + page — kept alive (mounted,
// hidden when inactive). Navigating inside one tab never affects another. Replaces AppShell as
// the App.tsx ShellGate element.
import { type ReactNode, useState } from "react";
import {
  MemoryRouter,
  Navigate,
  useRoutes,
  UNSAFE_LocationContext,
  UNSAFE_NavigationContext,
  UNSAFE_RouteContext,
  type RouteObject,
} from "react-router-dom";

import { useHasAccounts } from "@/lib/accountGate";
import { useFontScaleShortcuts } from "@/hooks/useFontScaleShortcuts";
import { useWorkbench } from "@/stores/workbench";
import { routeToPath } from "@/lib/workspaceRoute";
import RiskBanner from "@/components/layout/RiskBanner";
import { ToastViewport } from "@/components/ui/Toast";
import AiActivationPrompt from "@/components/ai/AiActivationPrompt";
import { workbenchRoutes } from "@/routes/workbenchRoutes";
import TabStrip from "./TabStrip";
import TabSwitcher from "./TabSwitcher";
import TabPane from "./TabPane";
import TabTitleSync from "./TabTitleSync";
import { useWorkbenchShortcuts } from "./useWorkbenchShortcuts";
import WorkbenchContextMenu from "./WorkbenchContextMenu";
import { useOpenInNewWindow } from "./useOpenInNewWindow";
import { useWorkbenchSession } from "./useWorkbenchSession";
import { useSingletonResponder } from "./useSingletonResponder";

/** Renders the matched route for one tab's MemoryRouter. */
function TabRoutes({ routes }: { routes: RouteObject[] }) {
  return useRoutes(routes);
}

// The default value React Router uses for RouteContext (no parent <Route>). Resetting
// to this makes the inner router a top-level router, not a descendant <Routes>.
const ROOT_ROUTE_CONTEXT = { outlet: null, matches: [], isDataRoute: false } as never;

/**
 * Each tab is a fully independent router (its own MemoryRouter + history). But the
 * whole app is mounted under App.tsx's data router (RouterProvider), so a bare
 * <MemoryRouter> here would be "a <Router> inside a <Router>" — which React Router
 * forbids. It throws an invariant that surfaces as a blank "Unexpected Application
 * Error!" screen on launch. Resetting the three router contexts to their defaults
 * turns each tab into an isolated router "island": the invariant sees no parent
 * router, and the tab's navigation / location / route matching stay independent of
 * the outer chrome router (which still drives the onboarding gate + risk banner).
 */
function IsolatedTabRouter({
  initialPath,
  children,
}: {
  initialPath: string;
  children: ReactNode;
}) {
  return (
    <UNSAFE_NavigationContext.Provider value={null as never}>
      <UNSAFE_LocationContext.Provider value={null as never}>
        <UNSAFE_RouteContext.Provider value={ROOT_ROUTE_CONTEXT}>
          <MemoryRouter initialEntries={[initialPath]}>{children}</MemoryRouter>
        </UNSAFE_RouteContext.Provider>
      </UNSAFE_LocationContext.Provider>
    </UNSAFE_NavigationContext.Provider>
  );
}

export interface WorkbenchShellProps {
  /** Route table for each tab. Defaults to the real app routes; tests inject stubs. */
  routes?: RouteObject[];
}

export default function WorkbenchShell({ routes = workbenchRoutes }: WorkbenchShellProps) {
  const hasAccounts = useHasAccounts();
  useFontScaleShortcuts();
  const tabs = useWorkbench((s) => s.tabs);
  const activeTabId = useWorkbench((s) => s.activeTabId);
  const closeTab = useWorkbench((s) => s.closeTab);
  const openInNewWindow = useOpenInNewWindow();
  const [switcherOpen, setSwitcherOpen] = useState(false);
  useWorkbenchShortcuts({ onOpenSwitcher: () => setSwitcherOpen(true) });

  // Boot + session restore (WB-23): detached window → its tab; main window → restore or Dashboard.
  useWorkbenchSession();
  // WB-17: answer cross-window singleton queries (focus this window if it holds the page).
  useSingletonResponder();

  if (!hasAccounts) return <Navigate to="/onboarding" replace />;

  return (
    <div className="flex h-full w-full flex-col overflow-hidden bg-parchment text-p10">
      {/* macOS transparent titlebar: a parchment drag strip that matches the tab strip, with
          the traffic lights resting on it (window: titleBarStyle "Transparent" + hiddenTitle). */}
      <div data-tauri-drag-region className="h-7 w-full shrink-0 bg-parchment" />
      {/* T4 risk banner — global, above the tabs, non-dismissable. */}
      <RiskBanner />
      <TabStrip
        onDetach={(tabId) => {
          const tab = useWorkbench.getState().tabs.find((t) => t.id === tabId);
          if (!tab) return;
          openInNewWindow({ route: tab.route, accountId: tab.accountId });
          closeTab(tabId);
        }}
      />
      <div className="relative min-h-0 w-full flex-1 overflow-hidden">
        {tabs.map((tab) => (
          <TabPane key={tab.id} active={tab.id === activeTabId}>
            {/* Each tab = an independent, context-isolated MemoryRouter (own Sidebar + pages),
                kept alive. Isolation (see IsolatedTabRouter) is what lets this nest under the
                App-level data router without tripping the "Router inside a Router" invariant. */}
            <IsolatedTabRouter initialPath={routeToPath(tab.route)}>
              <TabTitleSync tabId={tab.id} />
              <TabRoutes routes={routes} />
            </IsolatedTabRouter>
          </TabPane>
        ))}
      </div>
      <ToastViewport />
      <TabSwitcher open={switcherOpen} onClose={() => setSwitcherOpen(false)} />
      <WorkbenchContextMenu />
      <AiActivationPrompt />
    </div>
  );
}
