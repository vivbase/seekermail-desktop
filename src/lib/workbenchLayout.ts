// Session-restore serialization (WB-23, Model S). Converts the in-window tab structure to a
// plain, persistable shape and back. The on-disk persistence itself uses tauri-plugin-store
// (never localStorage) and the per-window save/load commands (02 §3 Module W) — that part is
// verified on the Mac; this module is the pure, testable (de)serialization it relies on.
import { deriveTitle, type TabState, type WorkspaceRoute } from "@/stores/workbench";

export interface WindowGeometry {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface PersistedTab {
  route: WorkspaceRoute;
  accountId?: string;
  pinned?: boolean;
}

export interface PersistedWindow {
  geometry?: WindowGeometry;
  tabs: PersistedTab[];
  activeIndex: number;
}

/** Serialize a window's tab structure (only durable bits — not scroll/draft, 18 §8). */
export function serializeLayout(
  tabs: TabState[],
  activeTabId: string | null,
  geometry?: WindowGeometry,
): PersistedWindow {
  const persistedTabs: PersistedTab[] = tabs.map((t) => ({
    route: t.route,
    ...(t.accountId ? { accountId: t.accountId } : {}),
    ...(t.pinned ? { pinned: true } : {}),
  }));
  const found = tabs.findIndex((t) => t.id === activeTabId);
  return {
    ...(geometry ? { geometry } : {}),
    tabs: persistedTabs,
    activeIndex: found < 0 ? 0 : found,
  };
}

let restoreSeq = 0;

/** Rehydrate a persisted layout into fresh TabState (new ids; titles re-derived). */
export function deserializeLayout(pw: PersistedWindow): {
  tabs: TabState[];
  activeTabId: string | null;
} {
  const now = Date.now();
  const tabs: TabState[] = pw.tabs.map((pt) => ({
    id: `restored_${++restoreSeq}`,
    route: pt.route,
    accountId: pt.accountId,
    pinned: pt.pinned,
    title: deriveTitle(pt.route),
    lastActiveAt: now,
  }));
  if (tabs.length === 0) return { tabs, activeTabId: null };
  const idx = Math.min(Math.max(pw.activeIndex, 0), tabs.length - 1);
  return { tabs, activeTabId: tabs[idx]?.id ?? null };
}
