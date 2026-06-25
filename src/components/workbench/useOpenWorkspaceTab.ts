// Open a workspace tab with singleton awareness (WB-05). The store already dedupes the
// global-singleton pages (Dashboard / Pending / Agent-IM) — it focuses the existing tab
// instead of opening a duplicate. This hook adds the user-facing toast that explains why
// no new tab appeared. Affordance call sites (the "+" button, WB-19 "open in new tab")
// should open through this hook. Cross-window singleton focus is WB-17.
import { useCallback } from "react";
import { useTranslation } from "react-i18next";

import {
  useWorkbench,
  SINGLETON_PAGES,
  MAX_TABS_PER_WINDOW,
  type TabSpec,
} from "@/stores/workbench";
import { showToast } from "@/components/ui/Toast";

export function useOpenWorkspaceTab(): (spec: TabSpec) => string {
  const { t } = useTranslation("common");
  return useCallback(
    (spec: TabSpec): string => {
      const isSingleton = SINGLETON_PAGES.includes(spec.route.page);
      const alreadyOpen =
        isSingleton && useWorkbench.getState().tabs.some((x) => x.route.page === spec.route.page);
      const id = useWorkbench.getState().openTab(spec);
      if (alreadyOpen) {
        showToast(t("wb_singleton_focused", "That page is already open — focused its tab"));
      } else if (useWorkbench.getState().tabs.length > MAX_TABS_PER_WINDOW) {
        showToast(t("wb_tab_limit", "Tab limit reached — unpin or close a tab first"));
      }
      return id;
    },
    [t],
  );
}
