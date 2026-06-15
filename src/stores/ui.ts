// Ephemeral UI state (07 §5). Client state ONLY — never holds data that came from
// a command (that's TanStack Query's job). Small + selector-based.
import { create } from "zustand";

export type Density = "comfortable" | "compact";
export type SearchMode = "keyword" | "semantic";

/** Pending-page filter chips (T081, root CLAUDE.md "Pending Page — Two Card Types"). */
export type PendingFilter = "all" | "decision" | "draft";

/** Report-page tabs (T089): risk events (T071) vs the E7 AI activity log. */
export type ReportTab = "risk" | "ai_activity";

/** L1 list filter (T039 folder/tag drawer). */
export interface MailFilter {
  folder: string | null;
  unreadOnly: boolean;
  starredOnly: boolean;
}

const DEFAULT_FILTER: MailFilter = { folder: null, unreadOnly: false, starredOnly: false };

interface UiState {
  /** Current route path, mirrored from the router to drive sidebar highlight. */
  activeRoute: string;
  /** Right agent rail open/closed. */
  agentRailOpen: boolean;
  /** List density. */
  density: Density;
  setActiveRoute: (route: string) => void;
  toggleAgentRail: () => void;
  setAgentRail: (open: boolean) => void;
  setDensity: (density: Density) => void;

  // ── Search mode (T034) — the /search page reads this; no overlay state. ──────
  searchMode: SearchMode;
  setSearchMode: (mode: SearchMode) => void;

  // ── Cross-account search filter (T113) ─────────────────────────────────────
  /** Selected account ids; empty = all accounts ("All Accounts" chip). */
  searchAccountFilter: string[];
  setSearchAccountFilter: (ids: string[]) => void;
  toggleSearchAccount: (id: string) => void;
  clearSearchAccountFilter: () => void;

  // ── Pending page filter chips (T081) ───────────────────────────────────────
  pendingFilter: PendingFilter;
  setPendingFilter: (filter: PendingFilter) => void;

  // ── Report page tabs (T089) ────────────────────────────────────────────────
  reportTab: ReportTab;
  setReportTab: (tab: ReportTab) => void;

  // ── L1 list filter (T039) ──────────────────────────────────────────────────
  mailFilter: MailFilter;
  setFolder: (folder: string | null) => void;
  setUnreadOnly: (on: boolean) => void;
  setStarredOnly: (on: boolean) => void;
  resetFilter: () => void;

  // ── Thread folding (T040) ──────────────────────────────────────────────────
  foldedThreads: Set<string>;
  toggleThreadFold: (threadId: string) => void;
  isThreadFolded: (threadId: string) => boolean;

  /** Whether thread-folding mode is active (toggle from FilterDrawer, T039/T040). */
  threadFoldingEnabled: boolean;
  setThreadFoldingEnabled: (on: boolean) => void;

  /** Which thread groups are currently expanded in fold mode (T040). */
  expandedThreadIds: Set<string>;
  toggleExpandedThread: (threadId: string) => void;
  isThreadExpanded: (threadId: string) => boolean;
}

export const useUi = create<UiState>((set, get) => ({
  activeRoute: "/",
  agentRailOpen: true,
  density: "comfortable",
  setActiveRoute: (route) => set({ activeRoute: route }),
  toggleAgentRail: () => set((s) => ({ agentRailOpen: !s.agentRailOpen })),
  setAgentRail: (open) => set({ agentRailOpen: open }),
  setDensity: (density) => set({ density }),

  searchMode: "semantic",
  setSearchMode: (mode) => set({ searchMode: mode }),

  searchAccountFilter: [],
  setSearchAccountFilter: (searchAccountFilter) => set({ searchAccountFilter }),
  toggleSearchAccount: (id) =>
    set((s) => {
      const has = s.searchAccountFilter.includes(id);
      return {
        searchAccountFilter: has
          ? s.searchAccountFilter.filter((x) => x !== id)
          : [...s.searchAccountFilter, id],
      };
    }),
  clearSearchAccountFilter: () => set({ searchAccountFilter: [] }),

  pendingFilter: "all",
  setPendingFilter: (pendingFilter) => set({ pendingFilter }),

  reportTab: "risk",
  setReportTab: (reportTab) => set({ reportTab }),

  mailFilter: DEFAULT_FILTER,
  setFolder: (folder) => set((s) => ({ mailFilter: { ...s.mailFilter, folder } })),
  setUnreadOnly: (unreadOnly) => set((s) => ({ mailFilter: { ...s.mailFilter, unreadOnly } })),
  setStarredOnly: (starredOnly) => set((s) => ({ mailFilter: { ...s.mailFilter, starredOnly } })),
  resetFilter: () => set({ mailFilter: DEFAULT_FILTER }),

  foldedThreads: new Set<string>(),
  toggleThreadFold: (threadId) =>
    set((s) => {
      const next = new Set(s.foldedThreads);
      if (next.has(threadId)) next.delete(threadId);
      else next.add(threadId);
      return { foldedThreads: next };
    }),
  isThreadFolded: (threadId) => get().foldedThreads.has(threadId),

  threadFoldingEnabled: false,
  setThreadFoldingEnabled: (on) => set({ threadFoldingEnabled: on }),

  expandedThreadIds: new Set<string>(),
  toggleExpandedThread: (threadId) =>
    set((s) => {
      const next = new Set(s.expandedThreadIds);
      if (next.has(threadId)) next.delete(threadId);
      else next.add(threadId);
      return { expandedThreadIds: next };
    }),
  isThreadExpanded: (threadId) => get().expandedThreadIds.has(threadId),
}));
