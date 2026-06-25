// Workbench keyboard shortcuts (WB-08). Registers a single window keydown listener:
//   Cmd/Ctrl+T          new tab
//   Cmd/Ctrl+Shift+T    reopen the last-closed tab
//   Cmd/Ctrl+W          close the active tab
//   Cmd/Ctrl+P          open the tab quick-switcher (Cmd+K is taken by Search — 18 §3 conflict,
//                       product to reconcile; see card WB-08)
//   Ctrl+Tab / Ctrl+Shift+Tab   cycle to the next / previous tab
import { useEffect } from "react";

import { useWorkbench, type TabSpec } from "@/stores/workbench";

export interface WorkbenchShortcutOptions {
  onNewTab?: () => void;
  onCloseActive?: () => void;
  onOpenSwitcher?: () => void;
  /** Default tab opened by Cmd/Ctrl+T when `onNewTab` is not given. */
  newTabSpec?: TabSpec;
}

const DEFAULT_NEW_TAB: TabSpec = { route: { page: "search" } };

export function useWorkbenchShortcuts(options: WorkbenchShortcutOptions = {}): void {
  const { onNewTab, onCloseActive, onOpenSwitcher, newTabSpec = DEFAULT_NEW_TAB } = options;

  useEffect(() => {
    const cycle = (dir: 1 | -1) => {
      const { tabs, activeTabId, activateTab } = useWorkbench.getState();
      if (tabs.length === 0) return;
      const i = tabs.findIndex((t) => t.id === activeTabId);
      const len = tabs.length;
      const next = tabs[((((i < 0 ? 0 : i) + dir) % len) + len) % len];
      if (next) activateTab(next.id);
    };

    const handler = (e: KeyboardEvent) => {
      // Ctrl+Tab / Ctrl+Shift+Tab — cycle (Ctrl specifically; the browser/OS standard).
      if (e.ctrlKey && e.key === "Tab") {
        e.preventDefault();
        cycle(e.shiftKey ? -1 : 1);
        return;
      }

      const mod = e.metaKey || e.ctrlKey;
      if (!mod) return;
      const k = e.key.toLowerCase();

      if (k === "t" && e.shiftKey) {
        e.preventDefault();
        useWorkbench.getState().reopenLastClosed();
      } else if (k === "t") {
        e.preventDefault();
        if (onNewTab) onNewTab();
        else useWorkbench.getState().openTab(newTabSpec);
      } else if (k === "w") {
        e.preventDefault();
        if (onCloseActive) onCloseActive();
        else {
          const id = useWorkbench.getState().activeTabId;
          if (id) useWorkbench.getState().closeTab(id);
        }
      } else if (k === "p") {
        e.preventDefault();
        onOpenSwitcher?.();
      }
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onNewTab, onCloseActive, onOpenSwitcher, newTabSpec]);
}
