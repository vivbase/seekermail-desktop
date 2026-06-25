// Close a tab with an unsaved-changes guard (WB-06). The store's closeTab is unconditional;
// this wrapper confirms first when the tab is dirty (a half-written compose/draft) so a tab
// is never closed silently. Returns whether the tab was actually closed.
import { useCallback } from "react";
import { useTranslation } from "react-i18next";

import { useWorkbench } from "@/stores/workbench";

export function useCloseTabWithGuard(): (tabId: string) => boolean {
  const { t } = useTranslation("common");
  return useCallback(
    (tabId: string): boolean => {
      const tab = useWorkbench.getState().tabs.find((x) => x.id === tabId);
      if (tab?.dirty) {
        const ok = window.confirm(
          t("wb_close_dirty_confirm", "This tab has unsaved changes. Close it anyway?"),
        );
        if (!ok) return false;
      }
      useWorkbench.getState().closeTab(tabId);
      return true;
    },
    [t],
  );
}
