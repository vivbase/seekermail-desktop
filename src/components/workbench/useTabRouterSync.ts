// Live integration glue (WB-09 v1): keep the top tab strip in sync with the existing
// React-Router pages, so tabs work on top of the real app without rewriting any page.
// URL ⇄ tabs, loop-free (every branch is guarded by `path === location.pathname`):
//   • URL change (sidebar nav / deep-link) → ensure + activate a tab for that route.
//   • Active-tab change (tab click / +) → navigate the router to that tab's route.
//
// This v1 reuses the router-driven pages via the existing <Outlet/> (so `useParams`, lazy
// loading, etc. keep working). True per-tab keep-alive is the WorkbenchRoot host (WB-02),
// layered in next once this is confirmed on real hardware.
import { useEffect } from "react";
import { useLocation, useNavigate } from "react-router-dom";

import { useWorkbench } from "@/stores/workbench";
import { routeToPath, pathToRoute } from "@/lib/workspaceRoute";

export function useTabRouterSync(): void {
  const location = useLocation();
  const navigate = useNavigate();
  const tabs = useWorkbench((s) => s.tabs);
  const activeTabId = useWorkbench((s) => s.activeTabId);
  const openTab = useWorkbench((s) => s.openTab);
  const activateTab = useWorkbench((s) => s.activateTab);

  // URL → tabs: a tab must exist and be active for the current route. Reads live state via
  // getState() so this effect depends ONLY on the path (no re-run on every tab change).
  useEffect(() => {
    const route = pathToRoute(location.pathname);
    if (!route) return;
    const state = useWorkbench.getState();
    const existing = state.tabs.find((t) => routeToPath(t.route) === location.pathname);
    if (existing) {
      if (state.activeTabId !== existing.id) activateTab(existing.id);
    } else {
      openTab({ route });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [location.pathname]);

  // Active tab → URL: navigate when the active tab's route differs from the URL.
  const activeTab = tabs.find((t) => t.id === activeTabId);
  const activePath = activeTab ? routeToPath(activeTab.route) : null;
  useEffect(() => {
    if (activePath && activePath !== location.pathname) navigate(activePath);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activePath]);
}
