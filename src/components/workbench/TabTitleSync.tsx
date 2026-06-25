// Mirrors a tab's OWN router location into the workbench store (WB-09 v2), so the tab strip's
// title/route follow what the user navigated to inside that tab. Updates the store only — it
// never navigates — so there is no loop with the tab's MemoryRouter.
import { useEffect } from "react";
import { useLocation } from "react-router-dom";

import { useWorkbench } from "@/stores/workbench";
import { routeToPath, pathToRoute } from "@/lib/workspaceRoute";

export default function TabTitleSync({ tabId }: { tabId: string }) {
  const location = useLocation();
  const navigateTab = useWorkbench((s) => s.navigateTab);
  useEffect(() => {
    const route = pathToRoute(location.pathname);
    if (!route) return;
    const tab = useWorkbench.getState().tabs.find((t) => t.id === tabId);
    if (tab && routeToPath(tab.route) !== location.pathname) {
      navigateTab(tabId, route);
    }
  }, [location.pathname, tabId, navigateTab]);
  return null;
}
