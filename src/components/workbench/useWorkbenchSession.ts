// Session boot + restore for the workbench (WB-23, Model S). Mounted once in WorkbenchShell.
//  - A detached window (`?boot=<token>`) boots to that one tab and is ephemeral: no persist,
//    no restore.
//  - The main window restores its persisted tab layout on launch (else Dashboard) and re-saves
//    it, debounced, on every structural change. Persistence rides the existing `ui.*` settings KV
//    under `ui.workbench_layout` — never localStorage (18 §8).
import { useEffect, useRef } from "react";

import { ipc, isTauri } from "@/ipc/client";
import { decodeBootToken } from "@/lib/bootToken";
import { deserializeLayout, serializeLayout, type PersistedWindow } from "@/lib/workbenchLayout";
import { useWorkbench } from "@/stores/workbench";

/** `app_settings` key (ui.* namespace) holding the main window's persisted layout. */
export const WORKBENCH_LAYOUT_KEY = "ui.workbench_layout";

const SAVE_DEBOUNCE_MS = 400;

function bootToken(): string | null {
  return new URLSearchParams(window.location.search).get("boot");
}

/** Load + parse the persisted layout. `null` when unset, off-Tauri, or malformed. */
export async function loadPersistedLayout(): Promise<ReturnType<typeof deserializeLayout> | null> {
  if (!isTauri()) return null;
  try {
    const raw = await ipc("get_setting", { key: WORKBENCH_LAYOUT_KEY });
    if (!raw) return null;
    const pw = JSON.parse(raw) as PersistedWindow;
    if (!pw || !Array.isArray(pw.tabs) || pw.tabs.length === 0) return null;
    return deserializeLayout(pw);
  } catch {
    return null;
  }
}

export function useWorkbenchSession(): void {
  const openTab = useWorkbench((s) => s.openTab);
  const restoreState = useWorkbench((s) => s.restoreState);
  const seeded = useRef(false);

  // One-time boot: detached window → its single tab; main window → restore, else Dashboard.
  useEffect(() => {
    if (seeded.current) return;
    seeded.current = true;
    if (useWorkbench.getState().tabs.length > 0) return;

    const boot = bootToken();
    if (boot) {
      openTab(decodeBootToken(boot) ?? { route: { page: "dashboard" } });
      return;
    }
    void loadPersistedLayout().then((restored) => {
      if (useWorkbench.getState().tabs.length > 0) return; // raced with another opener
      if (restored && restored.tabs.length > 0) {
        restoreState(restored.tabs, restored.activeTabId);
      } else {
        openTab({ route: { page: "dashboard" } });
      }
    });
  }, [openTab, restoreState]);

  // Persist the main window's layout on structural change, debounced. Detached windows never save.
  useEffect(() => {
    if (bootToken() || !isTauri()) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const unsub = useWorkbench.subscribe(() => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => {
        const { tabs, activeTabId } = useWorkbench.getState();
        const payload = serializeLayout(tabs, activeTabId);
        void ipc("set_setting", {
          key: WORKBENCH_LAYOUT_KEY,
          value: JSON.stringify(payload),
        }).catch(() => {
          /* persistence is best-effort */
        });
      }, SAVE_DEBOUNCE_MS);
    });
    return () => {
      if (timer) clearTimeout(timer);
      unsub();
    };
  }, []);
}
