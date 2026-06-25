// WB-21: a double-click handler factory. Double-clicking a surface (mail row, search result, …)
// opens it in a new TAB or a new WINDOW depending on the user's "Double-click opens" setting
// (default: new tab). Single-click keeps its existing in-place behavior.
import { useCallback } from "react";

import { useDoubleClickActionSetting } from "@/ipc/queries/settings";
import type { TabSpec } from "@/stores/workbench";

import { useOpenInNewWindow } from "./useOpenInNewWindow";
import { useOpenWorkspaceTab } from "./useOpenWorkspaceTab";

export function useOpenOnDoubleClick(): (spec: TabSpec) => void {
  const { action } = useDoubleClickActionSetting();
  const openTab = useOpenWorkspaceTab();
  const openWindow = useOpenInNewWindow();
  return useCallback(
    (spec: TabSpec) => {
      if (action === "window") openWindow(spec);
      else openTab(spec);
    },
    [action, openTab, openWindow],
  );
}
