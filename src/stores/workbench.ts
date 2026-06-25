// Workbench tab model (WB-01). Client state ONLY — never holds data that came from
// a command (that's TanStack Query's job). See spec: ../../seekermail-desktop-2026/docs/dev/
// 18_WORKBENCH_T1_T2_ENGINEERING_SPEC.md §2/§3 and 20 (WB-01).
//
// Model S: a TAB = a full app workspace. A tab carries the app's current page (`route`)
// plus its focused account; switching tabs swaps the whole app view. This store holds the
// in-window tab structure for ONE window (each window is its own React root).
import { create } from "zustand";

/** A page within the app + its params. A tab's CURRENT location (changes as the user navigates THIS tab). */
export interface WorkspaceRoute {
  page:
    | "dashboard"
    | "inbox"
    | "thread"
    | "compose"
    | "search"
    | "pending"
    | "agent_im"
    | "agents"
    | "repository"
    | "settings";
  params?: Record<string, string>; // e.g. { mailId } | { draftId } | { query }
}

/** A workspace tab. `title` is a display fallback; the render layer localises (account · page) via i18n. */
export interface TabState {
  id: string;
  route: WorkspaceRoute;
  accountId?: string; // focused account → sidebar account + tab-dot color
  title: string; // derived fallback label (see deriveTitle)
  pinned?: boolean;
  dirty?: boolean; // unsaved compose/draft → guards close/eviction
  lastActiveAt: number; // for LRU eviction
}

/** Input to openTab — a workspace to open. */
export interface TabSpec {
  route: WorkspaceRoute;
  accountId?: string;
}

/** Pages whose underlying DATA is a global singleton (one channel / one queue). Opening one
 *  focuses an existing tab showing it rather than spawning a duplicate (18 §2/§9). */
export const SINGLETON_PAGES: WorkspaceRoute["page"][] = ["dashboard", "pending", "agent_im"];

/** Soft cap; full-app tabs are heavier than content tabs, so this is lower than a browser's (18 §3/§10). */
export const MAX_TABS_PER_WINDOW = 8;

/** Bounded "reopen closed tab" history. */
export const RECENTLY_CLOSED_LIMIT = 10;

const PAGE_LABEL: Record<WorkspaceRoute["page"], string> = {
  dashboard: "Dashboard",
  inbox: "Inbox",
  thread: "Reading",
  compose: "Compose",
  search: "Search",
  pending: "Pending",
  agent_im: "Team",
  agents: "Agents",
  repository: "Repository",
  settings: "Settings",
};

/** Display fallback for a tab title. The render layer may prepend the account name and localise. */
export function deriveTitle(route: WorkspaceRoute): string {
  return PAGE_LABEL[route.page] ?? "SeekerMail";
}

/** LRU eviction candidate: the oldest tab that is neither pinned, dirty, nor active. `null` if all protected. */
export function pickEvictionCandidate(
  tabs: TabState[],
  activeTabId: string | null,
): TabState | null {
  let candidate: TabState | null = null;
  for (const t of tabs) {
    if (t.pinned || t.dirty || t.id === activeTabId) continue;
    if (candidate === null || t.lastActiveAt < candidate.lastActiveAt) candidate = t;
  }
  return candidate;
}

let _tabSeq = 0;
const newTabId = (): string => `tab_${++_tabSeq}`;
const now = (): number => Date.now();

export interface WorkbenchState {
  tabs: TabState[]; // order = tab-strip order
  activeTabId: string | null;
  recentlyClosed: TabState[]; // bounded stack, newest last

  /** Open a NEW workspace tab. Global-singleton pages focus an existing tab instead of duplicating.
   *  Returns the active tab id. Enforces the LRU cap (non-pinned, non-dirty, non-active evicted). */
  openTab: (spec: TabSpec) => string;
  /** Close a tab; reassigns the active tab to a neighbour; pushes the closed tab to recentlyClosed. */
  closeTab: (tabId: string) => void;
  /** Make a tab active. */
  activateTab: (tabId: string) => void;
  /** Navigate WITHIN one tab — changes only that tab's route (and title). */
  navigateTab: (tabId: string, route: WorkspaceRoute) => void;
  /** Reorder a tab to a new index in the strip. */
  moveTab: (tabId: string, toIndex: number) => void;
  setPinned: (tabId: string, pinned: boolean) => void;
  setDirty: (tabId: string, dirty: boolean) => void;
  /** Reopen the most recently closed tab. No-op if the history is empty. */
  reopenLastClosed: () => void;
  /** Replace the whole tab set on launch from a persisted layout (WB-23). Skips singleton
   *  dedupe / LRU cap — the saved layout was already valid when it was written. */
  restoreState: (tabs: TabState[], activeTabId: string | null) => void;
}

export const useWorkbench = create<WorkbenchState>((set, get) => ({
  tabs: [],
  activeTabId: null,
  recentlyClosed: [],

  openTab: (spec) => {
    const state = get();

    // Singleton dedupe: focus an existing tab showing this global-singleton page.
    if (SINGLETON_PAGES.includes(spec.route.page)) {
      const existing = state.tabs.find((t) => t.route.page === spec.route.page);
      if (existing) {
        set({
          activeTabId: existing.id,
          tabs: state.tabs.map((t) => (t.id === existing.id ? { ...t, lastActiveAt: now() } : t)),
        });
        return existing.id;
      }
    }

    const tab: TabState = {
      id: newTabId(),
      route: spec.route,
      accountId: spec.accountId,
      title: deriveTitle(spec.route),
      lastActiveAt: now(),
    };
    let tabs = [...state.tabs, tab];

    // LRU cap: evict the oldest non-pinned, non-dirty, non-active tab (skip if all protected).
    let recentlyClosed = state.recentlyClosed;
    if (tabs.length > MAX_TABS_PER_WINDOW) {
      const victim = pickEvictionCandidate(tabs, tab.id);
      if (victim) {
        tabs = tabs.filter((t) => t.id !== victim.id);
        recentlyClosed = [...state.recentlyClosed, victim].slice(-RECENTLY_CLOSED_LIMIT);
      }
    }

    set({ tabs, activeTabId: tab.id, recentlyClosed });
    return tab.id;
  },

  closeTab: (tabId) => {
    const state = get();
    const index = state.tabs.findIndex((t) => t.id === tabId);
    if (index < 0) return;

    const closed = state.tabs[index];
    if (closed === undefined) return;
    const tabs = state.tabs.filter((t) => t.id !== tabId);

    let activeTabId = state.activeTabId;
    if (state.activeTabId === tabId) {
      const neighbor = tabs[Math.max(0, index - 1)];
      activeTabId = neighbor ? neighbor.id : null;
    }

    set({
      tabs,
      activeTabId,
      recentlyClosed: [...state.recentlyClosed, closed].slice(-RECENTLY_CLOSED_LIMIT),
    });
  },

  activateTab: (tabId) => {
    const state = get();
    if (!state.tabs.some((t) => t.id === tabId)) return;
    set({
      activeTabId: tabId,
      tabs: state.tabs.map((t) => (t.id === tabId ? { ...t, lastActiveAt: now() } : t)),
    });
  },

  navigateTab: (tabId, route) => {
    set((state) => ({
      tabs: state.tabs.map((t) =>
        t.id === tabId ? { ...t, route, title: deriveTitle(route) } : t,
      ),
    }));
  },

  moveTab: (tabId, toIndex) => {
    const state = get();
    const from = state.tabs.findIndex((t) => t.id === tabId);
    if (from < 0) return;
    const clamped = Math.max(0, Math.min(toIndex, state.tabs.length - 1));
    if (from === clamped) return;
    const tabs = [...state.tabs];
    const [moved] = tabs.splice(from, 1);
    if (moved === undefined) return;
    tabs.splice(clamped, 0, moved);
    set({ tabs });
  },

  setPinned: (tabId, pinned) => {
    set((state) => ({
      tabs: state.tabs.map((t) => (t.id === tabId ? { ...t, pinned } : t)),
    }));
  },

  setDirty: (tabId, dirty) => {
    set((state) => ({
      tabs: state.tabs.map((t) => (t.id === tabId ? { ...t, dirty } : t)),
    }));
  },

  reopenLastClosed: () => {
    const state = get();
    if (state.recentlyClosed.length === 0) return;
    const recentlyClosed = [...state.recentlyClosed];
    const restored = recentlyClosed.pop() as TabState;
    const tab: TabState = { ...restored, id: newTabId(), lastActiveAt: now() };
    set({ tabs: [...state.tabs, tab], activeTabId: tab.id, recentlyClosed });
  },

  restoreState: (tabs, activeTabId) => {
    set({ tabs, activeTabId: activeTabId ?? tabs[0]?.id ?? null });
  },
}));
