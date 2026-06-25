// Global "open in new tab" right-click menu (WB-19). One instance, mounted in WorkbenchShell.
// Listens for contextmenu anywhere; if the click is on (or inside) an element carrying
// `data-open-spec`, it shows our menu instead of the OS/webview default and can open that
// workspace in a new tab. ("Open in new window" arrives with the T2 Rust window command.)
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { parseOpenSpec, OPEN_SPEC_ATTR } from "@/lib/openSpec";
import { currentUiScale } from "@/lib/fontScale";
import type { TabSpec } from "@/stores/workbench";
import { useOpenSingletonRouted } from "./useOpenSingletonRouted";
import { useOpenInNewWindow } from "./useOpenInNewWindow";

interface MenuState {
  x: number;
  y: number;
  spec: TabSpec;
}

export default function WorkbenchContextMenu() {
  const { t } = useTranslation("common");
  const openWorkspaceTab = useOpenSingletonRouted();
  const openInNewWindow = useOpenInNewWindow();
  const [menu, setMenu] = useState<MenuState | null>(null);

  // Open the menu on right-click over a marked element.
  useEffect(() => {
    const onContext = (e: MouseEvent) => {
      const el = (e.target as HTMLElement | null)?.closest?.(`[${OPEN_SPEC_ATTR}]`);
      const spec = parseOpenSpec(el?.getAttribute(OPEN_SPEC_ATTR));
      if (!spec) return; // not a workbench surface — leave the default menu alone
      e.preventDefault();
      // #root is zoomed by --ui-scale, which also scales a fixed element's offset, so map the
      // viewport cursor coords into that zoomed space to open the menu AT the pointer.
      const scale = currentUiScale();
      setMenu({ x: e.clientX / scale, y: e.clientY / scale, spec });
    };
    document.addEventListener("contextmenu", onContext);
    return () => document.removeEventListener("contextmenu", onContext);
  }, []);

  // Dismiss on outside click / Escape.
  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setMenu(null);
    };
    window.addEventListener("click", close);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [menu]);

  if (!menu) return null;

  return (
    <ul
      role="menu"
      style={{ left: menu.x, top: menu.y }}
      className="fixed z-50 min-w-[180px] overflow-hidden rounded-[8px] border border-divider bg-surface py-1 shadow-card"
      onClick={(e) => e.stopPropagation()}
    >
      <li role="none">
        <button
          role="menuitem"
          type="button"
          className="block w-full px-3 py-1.5 text-left font-ui text-[12px] text-p9 hover:bg-parchment hover:text-p10"
          onClick={() => {
            openWorkspaceTab(menu.spec);
            setMenu(null);
          }}
        >
          {t("wb_open_in_new_tab", "Open in new tab")}
        </button>
      </li>
      <li role="none">
        <button
          role="menuitem"
          type="button"
          className="block w-full px-3 py-1.5 text-left font-ui text-[12px] text-p9 hover:bg-parchment hover:text-p10"
          onClick={() => {
            openInNewWindow(menu.spec);
            setMenu(null);
          }}
        >
          {t("wb_open_in_new_window", "Open in new window")}
        </button>
      </li>
    </ul>
  );
}
