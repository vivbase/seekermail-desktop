// WB-17: open a workspace, routing global-singleton pages across windows. If another window
// already shows the page it focuses itself and we stand down; otherwise we open it here. Non-
// singleton specs open immediately through the normal tab opener.
import { useCallback } from "react";

import { requestCrossWindowSingleton } from "@/ipc/singletonBroadcast";
import { SINGLETON_PAGES, type TabSpec } from "@/stores/workbench";

import { useOpenWorkspaceTab } from "./useOpenWorkspaceTab";

export function useOpenSingletonRouted(): (spec: TabSpec) => void {
  const openTab = useOpenWorkspaceTab();
  return useCallback(
    (spec: TabSpec) => {
      if (!SINGLETON_PAGES.includes(spec.route.page)) {
        openTab(spec);
        return;
      }
      void requestCrossWindowSingleton(spec.route.page).then((claimedElsewhere) => {
        if (!claimedElsewhere) openTab(spec);
      });
    },
    [openTab],
  );
}
