// Keep-alive Tab Host (WB-02). Renders ONE TabPane per open tab from the workbench store,
// keeping every open tab mounted (only the active one is visible). The per-tab workspace —
// a full AppShell (sidebar + main-by-route), Model S — is supplied via `renderWorkspace`,
// wired to the real shell in WB-09 when App.tsx is migrated. This component owns only the
// keep-alive host; the tab strip is WB-03.
import { type ReactNode } from "react";

import { useWorkbench, type TabState } from "@/stores/workbench";
import TabPane from "./TabPane";

export interface WorkbenchTabHostProps {
  /** Render the full app workspace for a tab (sidebar + main by `tab.route`). */
  renderWorkspace: (tab: TabState) => ReactNode;
}

export default function WorkbenchTabHost({ renderWorkspace }: WorkbenchTabHostProps) {
  const tabs = useWorkbench((s) => s.tabs);
  const activeTabId = useWorkbench((s) => s.activeTabId);

  return (
    <div className="relative h-full min-h-0 w-full flex-1">
      {tabs.map((tab) => (
        <TabPane key={tab.id} active={tab.id === activeTabId}>
          {renderWorkspace(tab)}
        </TabPane>
      ))}
    </div>
  );
}
