// Tab-driven workbench root (WB-09 mechanism, Model S). Replaces "the router Outlet renders
// one page" with "a top tab strip over a keep-alive host, where each tab renders a full app
// view (sidebar + main-by-route)". The ACTIVE tab's route mirrors to the URL (deep-link).
//
// Scope here = the mechanism, one-way (tab route → URL), ref-guarded so it never loops.
// The reverse direction (browser back/forward → switch the active tab's route) needs a POP
// listener over real history and is wired + verified at integration on the Mac.
//
// Integration (the live App.tsx swap) is staged: App.tsx provides the real `pages` registry
// (the existing lazy route components) and `renderSidebar` (the real Sidebar wired to
// navigateTab), inside the existing RouterProvider + ShellGate. A small, reviewable,
// regression-guarded change verified in a real browser.
import { type ComponentType, type ReactNode, useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";

import { useWorkbench, type WorkspaceRoute, type TabState } from "@/stores/workbench";
import { routeToPath } from "@/lib/workspaceRoute";
import { type AccountColorToken } from "@/lib/accountColor";
import TabStrip from "./TabStrip";
import WorkbenchTabHost from "./WorkbenchTabHost";
import TabSwitcher from "./TabSwitcher";
import { useWorkbenchShortcuts } from "./useWorkbenchShortcuts";
import { useCloseTabWithGuard } from "./useCloseTab";

export type WorkspacePageComponent = ComponentType<{ tab: TabState }>;
export type WorkspacePages = Partial<Record<WorkspaceRoute["page"], WorkspacePageComponent>>;

export interface WorkbenchRootProps {
  /** page → component registry. App.tsx wires the real lazy route pages here (WB-09 integration). */
  pages: WorkspacePages;
  /** Global risk layer (the existing RiskBanner) — rendered ABOVE the tab strip (WB-10, load-bearing). */
  riskLayer?: ReactNode;
  /** Per-tab sidebar (the real Sidebar driving navigateTab is wired at integration). */
  renderSidebar?: (tab: TabState) => ReactNode;
  accentFor?: (accountId?: string) => AccountColorToken | undefined;
  onNewTab?: () => void;
  onDetach?: (tabId: string) => void;
}

/** One-way mirror: when the ACTIVE tab's route changes, reflect it in the URL (18 §3). */
function useActiveTabUrlMirror(): void {
  const navigate = useNavigate();
  const tabs = useWorkbench((s) => s.tabs);
  const activeTabId = useWorkbench((s) => s.activeTabId);
  const activeTab = tabs.find((t) => t.id === activeTabId);
  const activePath = activeTab ? routeToPath(activeTab.route) : null;
  const lastPushed = useRef<string | null>(null);

  useEffect(() => {
    if (activePath && activePath !== lastPushed.current) {
      lastPushed.current = activePath;
      navigate(activePath);
    }
  }, [activePath, navigate]);
}

export default function WorkbenchRoot({
  pages,
  renderSidebar,
  accentFor,
  onNewTab,
  onDetach,
  riskLayer,
}: WorkbenchRootProps) {
  useActiveTabUrlMirror();
  const [switcherOpen, setSwitcherOpen] = useState(false);
  const closeWithGuard = useCloseTabWithGuard();
  useWorkbenchShortcuts({
    onOpenSwitcher: () => setSwitcherOpen(true),
    onCloseActive: () => {
      const id = useWorkbench.getState().activeTabId;
      if (id) closeWithGuard(id);
    },
  });

  return (
    <div className="flex h-full w-full flex-col overflow-hidden bg-parchment text-p10">
      {/* WB-10: the T4 risk layer sits ABOVE the tab strip — non-dismissable, never hidden by a tab. */}
      {riskLayer}
      <TabStrip accentFor={accentFor} onNewTab={onNewTab} onDetach={onDetach} />
      <WorkbenchTabHost
        renderWorkspace={(tab) => {
          const Page = pages[tab.route.page];
          return (
            <div className="app flex min-h-0 w-full flex-1 overflow-hidden">
              {renderSidebar?.(tab)}
              <main className="min-h-0 flex-1 overflow-auto bg-parchment">
                {Page ? <Page tab={tab} /> : null}
              </main>
            </div>
          );
        }}
      />
      <TabSwitcher open={switcherOpen} onClose={() => setSwitcherOpen(false)} />
    </div>
  );
}
