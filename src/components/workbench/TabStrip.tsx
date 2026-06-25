// Top tab strip (WB-03) + interaction wiring (WB-04) + pin/close menu (WB-07) + guards (WB-06).
// Browser-style, sits at the top of the window above the shell (Model S). Click = activate,
// × = close (dirty-guarded; focus moves to a neighbour), + = new tab (warns at the cap), drag
// = reorder WITHIN the strip (cross-window drag is T3). Right-click = pin/unpin/close menu;
// pinned tabs render leftmost. Account-color dots are resolved via `accentFor` (WB-09).
import { useRef, useState, type DragEvent, type MouseEvent } from "react";
import { Plus } from "lucide-react";
import { useTranslation } from "react-i18next";

import { useWorkbench, MAX_TABS_PER_WINDOW, type TabSpec } from "@/stores/workbench";
import { type AccountColorToken } from "@/lib/accountColor";
import { showToast } from "@/components/ui/Toast";
import { cn } from "@/lib/cn";
import { currentUiScale } from "@/lib/fontScale";
import Tab from "./Tab";
import { useCloseTabWithGuard } from "./useCloseTab";

export interface TabStripProps {
  accentFor?: (accountId?: string) => AccountColorToken | undefined;
  onNewTab?: () => void;
  onDetach?: (tabId: string) => void;
  newTabSpec?: TabSpec;
}

const DEFAULT_NEW_TAB: TabSpec = { route: { page: "search" } };
const MENU_ITEM =
  "block w-full px-3 py-1.5 text-left font-ui text-[11px] text-p9 hover:bg-parchment hover:text-p10";

export default function TabStrip({
  accentFor,
  onNewTab,
  onDetach,
  newTabSpec = DEFAULT_NEW_TAB,
}: TabStripProps) {
  const { t } = useTranslation("common");
  const tabs = useWorkbench((s) => s.tabs);
  const activeTabId = useWorkbench((s) => s.activeTabId);
  const activateTab = useWorkbench((s) => s.activateTab);
  const moveTab = useWorkbench((s) => s.moveTab);
  const setPinned = useWorkbench((s) => s.setPinned);
  const openTab = useWorkbench((s) => s.openTab);
  const closeWithGuard = useCloseTabWithGuard();

  const dragId = useRef<string | null>(null);
  const tabEls = useRef<Map<string, HTMLDivElement>>(new Map());
  const [menu, setMenu] = useState<{ tabId: string; x: number; y: number } | null>(null);

  // Pinned tabs render leftmost; otherwise keep the store order (stable sort).
  const ordered = [...tabs].sort((a, b) => Number(b.pinned ?? false) - Number(a.pinned ?? false));
  const menuTab = menu ? tabs.find((x) => x.id === menu.tabId) : undefined;

  const handleClose = (id: string) => {
    if (closeWithGuard(id)) {
      const next = useWorkbench.getState().activeTabId;
      if (next) tabEls.current.get(next)?.focus();
    }
  };

  const handleDrop = (targetId: string) => {
    const from = dragId.current;
    dragId.current = null;
    if (!from || from === targetId) return;
    const toIndex = useWorkbench.getState().tabs.findIndex((x) => x.id === targetId);
    if (toIndex >= 0) moveTab(from, toIndex);
  };

  const handleNewTab = () => {
    if (onNewTab) onNewTab();
    else openTab(newTabSpec);
    if (useWorkbench.getState().tabs.length > MAX_TABS_PER_WINDOW) {
      showToast(t("wb_tab_limit", "Tab limit reached — unpin or close a tab first"));
    }
  };

  const openMenu = (tabId: string, e: MouseEvent<HTMLDivElement>) => {
    e.preventDefault();
    // Map viewport coords into the --ui-scale-zoomed space so the menu opens at the cursor.
    const scale = currentUiScale();
    setMenu({ tabId, x: e.clientX / scale, y: e.clientY / scale });
  };

  return (
    <div
      role="tablist"
      aria-label={t("wb_tabs", "Open tabs")}
      className="flex h-9 w-full items-end gap-0.5 overflow-x-auto border-b border-divider bg-parchment px-1.5 pt-1.5 [scrollbar-width:none]"
    >
      {ordered.map((tab) => (
        <Tab
          key={tab.id}
          title={tab.title}
          active={tab.id === activeTabId}
          colorToken={accentFor?.(tab.accountId)}
          dirty={tab.dirty}
          pinned={tab.pinned}
          closeLabel={t("wb_close_tab", "Close tab")}
          detachLabel={t("wb_open_in_new_window", "Open in new window")}
          onActivate={() => activateTab(tab.id)}
          onClose={() => handleClose(tab.id)}
          onDetach={onDetach ? () => onDetach(tab.id) : undefined}
          onContextMenu={(e) => openMenu(tab.id, e)}
          draggable
          onDragStart={() => {
            dragId.current = tab.id;
          }}
          onDragOver={(e: DragEvent<HTMLDivElement>) => e.preventDefault()}
          onDrop={() => handleDrop(tab.id)}
          tabRef={(el) => {
            if (el) tabEls.current.set(tab.id, el);
            else tabEls.current.delete(tab.id);
          }}
        />
      ))}
      <button
        type="button"
        aria-label={t("wb_new_tab", "New tab")}
        onClick={handleNewTab}
        className="mb-0.5 ml-0.5 grid h-7 w-7 shrink-0 place-items-center rounded-[6px] text-p7 transition-colors hover:bg-surface hover:text-p10 focus-visible:ring-2 focus-visible:ring-slate"
      >
        <Plus className="h-4 w-4" />
      </button>

      {menu && menuTab ? (
        <div
          className="fixed inset-0 z-40"
          onClick={() => setMenu(null)}
          onContextMenu={(e) => {
            e.preventDefault();
            setMenu(null);
          }}
        >
          <ul
            role="menu"
            style={{ left: menu.x, top: menu.y }}
            className="absolute min-w-[150px] overflow-hidden rounded-[8px] border border-divider bg-surface py-1 shadow-card"
            onClick={(e) => e.stopPropagation()}
          >
            <li role="none">
              <button
                role="menuitem"
                type="button"
                className={MENU_ITEM}
                onClick={() => {
                  setPinned(menu.tabId, !menuTab.pinned);
                  setMenu(null);
                }}
              >
                {menuTab.pinned ? t("wb_unpin", "Unpin tab") : t("wb_pin", "Pin tab")}
              </button>
            </li>
            {onDetach ? (
              <li role="none">
                <button
                  role="menuitem"
                  type="button"
                  className={MENU_ITEM}
                  onClick={() => {
                    const id = menu.tabId;
                    setMenu(null);
                    onDetach(id);
                  }}
                >
                  {t("wb_open_in_new_window", "Open in new window")}
                </button>
              </li>
            ) : null}
            <li role="none">
              <button
                role="menuitem"
                type="button"
                className={cn(MENU_ITEM, "hover:!bg-red hover:!text-white")}
                onClick={() => {
                  const id = menu.tabId;
                  setMenu(null);
                  handleClose(id);
                }}
              >
                {t("wb_close_tab", "Close tab")}
              </button>
            </li>
          </ul>
        </div>
      ) : null}
    </div>
  );
}
