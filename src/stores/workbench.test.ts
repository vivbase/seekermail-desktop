import { describe, it, expect, beforeEach } from "vitest";

import {
  useWorkbench,
  deriveTitle,
  pickEvictionCandidate,
  MAX_TABS_PER_WINDOW,
  type TabState,
} from "./workbench";

/** Build a fully-formed tab for setState fixtures. */
const mk = (id: string, lastActiveAt: number, extra: Partial<TabState> = {}): TabState => ({
  id,
  route: { page: "inbox" },
  title: "Inbox",
  lastActiveAt,
  ...extra,
});

describe("workbench store (WB-01)", () => {
  beforeEach(() => {
    useWorkbench.setState({ tabs: [], activeTabId: null, recentlyClosed: [] });
  });

  it("opens a tab and makes it active", () => {
    const id = useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "acct1" });
    const s = useWorkbench.getState();
    expect(s.tabs).toHaveLength(1);
    expect(s.activeTabId).toBe(id);
    expect(s.tabs[0]?.route.page).toBe("inbox");
    expect(s.tabs[0]?.accountId).toBe("acct1");
    expect(s.tabs[0]?.title).toBe("Inbox");
  });

  it("dedupes global-singleton pages (focuses the existing tab)", () => {
    const first = useWorkbench.getState().openTab({ route: { page: "dashboard" } });
    const second = useWorkbench.getState().openTab({ route: { page: "dashboard" } });
    const s = useWorkbench.getState();
    expect(first).toBe(second);
    expect(s.tabs.filter((t) => t.route.page === "dashboard")).toHaveLength(1);
    expect(s.activeTabId).toBe(first);
  });

  it("allows multiple non-singleton tabs of the same page", () => {
    useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "a" });
    useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "b" });
    expect(useWorkbench.getState().tabs).toHaveLength(2);
  });

  it("closes a tab, reassigns active to a neighbour, and records it", () => {
    const a = useWorkbench.getState().openTab({ route: { page: "inbox" } });
    const b = useWorkbench.getState().openTab({ route: { page: "compose" } });
    useWorkbench.getState().closeTab(b);
    const s = useWorkbench.getState();
    expect(s.tabs).toHaveLength(1);
    expect(s.activeTabId).toBe(a);
    expect(s.recentlyClosed.at(-1)?.id).toBe(b);
  });

  it("sets active to null when the last tab closes", () => {
    const a = useWorkbench.getState().openTab({ route: { page: "inbox" } });
    useWorkbench.getState().closeTab(a);
    expect(useWorkbench.getState().activeTabId).toBeNull();
  });

  it("reopens the most recently closed tab", () => {
    const a = useWorkbench.getState().openTab({ route: { page: "search" } });
    useWorkbench.getState().closeTab(a);
    useWorkbench.getState().reopenLastClosed();
    const s = useWorkbench.getState();
    expect(s.tabs).toHaveLength(1);
    expect(s.tabs[0]?.route.page).toBe("search");
    expect(s.recentlyClosed).toHaveLength(0);
  });

  it("navigates within one tab only (per-tab routing)", () => {
    const a = useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "x" });
    const b = useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "y" });
    useWorkbench.getState().navigateTab(a, { page: "compose" });
    const s = useWorkbench.getState();
    expect(s.tabs.find((t) => t.id === a)?.route.page).toBe("compose");
    expect(s.tabs.find((t) => t.id === a)?.title).toBe("Compose");
    expect(s.tabs.find((t) => t.id === b)?.route.page).toBe("inbox");
  });

  it("reorders tabs", () => {
    const a = useWorkbench.getState().openTab({ route: { page: "inbox" } });
    const b = useWorkbench.getState().openTab({ route: { page: "search" } });
    const c = useWorkbench.getState().openTab({ route: { page: "compose" } });
    useWorkbench.getState().moveTab(c, 0);
    expect(useWorkbench.getState().tabs.map((t) => t.id)).toEqual([c, a, b]);
  });

  it("sets pinned and dirty flags", () => {
    const a = useWorkbench.getState().openTab({ route: { page: "compose" } });
    useWorkbench.getState().setPinned(a, true);
    useWorkbench.getState().setDirty(a, true);
    const tab = useWorkbench.getState().tabs.find((t) => t.id === a);
    expect(tab?.pinned).toBe(true);
    expect(tab?.dirty).toBe(true);
  });

  describe("LRU eviction", () => {
    it("pickEvictionCandidate skips pinned/dirty/active and picks the oldest", () => {
      const tabs = [mk("t0", 100), mk("t1", 50, { pinned: true }), mk("t2", 75)];
      expect(pickEvictionCandidate(tabs, null)?.id).toBe("t2"); // t1 pinned → excluded; oldest of {t0,t2} is t2
      expect(pickEvictionCandidate(tabs, "t2")?.id).toBe("t0"); // t2 active → excluded
    });

    it("returns null when every tab is protected", () => {
      const tabs = [mk("t0", 100, { pinned: true }), mk("t1", 50, { dirty: true })];
      expect(pickEvictionCandidate(tabs, null)).toBeNull();
    });

    it("evicts the oldest non-protected tab when over the cap", () => {
      const tabs = Array.from({ length: MAX_TABS_PER_WINDOW }, (_, i) => mk(`t${i}`, 1000 + i));
      useWorkbench.setState({ tabs, activeTabId: "t7", recentlyClosed: [] });
      useWorkbench.getState().openTab({ route: { page: "search" } });
      const s = useWorkbench.getState();
      expect(s.tabs).toHaveLength(MAX_TABS_PER_WINDOW); // cap held
      expect(s.tabs.find((t) => t.id === "t0")).toBeUndefined(); // oldest evicted
      expect(s.recentlyClosed.at(-1)?.id).toBe("t0");
    });

    it("does not evict a pinned oldest tab", () => {
      const tabs = Array.from({ length: MAX_TABS_PER_WINDOW }, (_, i) =>
        mk(`t${i}`, 1000 + i, i === 0 ? { pinned: true } : {}),
      );
      useWorkbench.setState({ tabs, activeTabId: "t7", recentlyClosed: [] });
      useWorkbench.getState().openTab({ route: { page: "search" } });
      const s = useWorkbench.getState();
      expect(s.tabs.find((t) => t.id === "t0")).toBeDefined(); // pinned survives
      expect(s.tabs.find((t) => t.id === "t1")).toBeUndefined(); // next-oldest evicted
    });

    it("keeps all tabs when every candidate is protected", () => {
      const tabs = Array.from({ length: MAX_TABS_PER_WINDOW }, (_, i) =>
        mk(`t${i}`, 1000 + i, { pinned: true }),
      );
      useWorkbench.setState({ tabs, activeTabId: "t7", recentlyClosed: [] });
      useWorkbench.getState().openTab({ route: { page: "search" } });
      expect(useWorkbench.getState().tabs).toHaveLength(MAX_TABS_PER_WINDOW + 1); // nothing evictable
    });
  });

  it("deriveTitle maps pages to display labels", () => {
    expect(deriveTitle({ page: "inbox" })).toBe("Inbox");
    expect(deriveTitle({ page: "agent_im" })).toBe("Team");
    expect(deriveTitle({ page: "thread" })).toBe("Reading");
  });

  it("restoreState replaces the whole tab set and resolves the active tab (WB-23)", () => {
    const tabs = [mk("a", 1), mk("b", 2), mk("c", 3)];
    useWorkbench.getState().restoreState(tabs, "b");
    let s = useWorkbench.getState();
    expect(s.tabs.map((t) => t.id)).toEqual(["a", "b", "c"]);
    expect(s.activeTabId).toBe("b");
    // A null active id falls back to the first tab.
    useWorkbench.getState().restoreState(tabs, null);
    s = useWorkbench.getState();
    expect(s.activeTabId).toBe("a");
  });
});
