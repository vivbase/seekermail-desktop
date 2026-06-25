import { describe, it, expect } from "vitest";

import { serializeLayout, deserializeLayout, type PersistedWindow } from "./workbenchLayout";
import type { TabState, WorkspaceRoute } from "@/stores/workbench";

const mk = (id: string, page: WorkspaceRoute["page"], extra: Partial<TabState> = {}): TabState => ({
  id,
  route: { page },
  title: "X",
  lastActiveAt: 0,
  ...extra,
});

describe("workbenchLayout (WB-23 session restore)", () => {
  it("serializes the durable tab structure + active index", () => {
    const tabs = [mk("a", "inbox", { accountId: "x", pinned: true }), mk("b", "search")];
    const pw = serializeLayout(tabs, "b", { x: 0, y: 0, w: 800, h: 600 });
    expect(pw.activeIndex).toBe(1);
    expect(pw.geometry).toEqual({ x: 0, y: 0, w: 800, h: 600 });
    expect(pw.tabs).toEqual([
      { route: { page: "inbox" }, accountId: "x", pinned: true },
      { route: { page: "search" } },
    ]);
  });

  it("deserializes into fresh tabs with the active one selected and titles re-derived", () => {
    const pw: PersistedWindow = {
      tabs: [{ route: { page: "dashboard" } }, { route: { page: "inbox" }, accountId: "y" }],
      activeIndex: 1,
    };
    const { tabs, activeTabId } = deserializeLayout(pw);
    expect(tabs).toHaveLength(2);
    expect(tabs[1]?.accountId).toBe("y");
    expect(tabs[1]?.title).toBe("Inbox");
    expect(activeTabId).toBe(tabs[1]?.id);
  });

  it("round-trips routes/pinned (ids are regenerated)", () => {
    const tabs = [mk("a", "agent_im", { pinned: true }), mk("b", "compose", { accountId: "w" })];
    const { tabs: restored } = deserializeLayout(serializeLayout(tabs, "a"));
    expect(restored.map((t) => t.route.page)).toEqual(["agent_im", "compose"]);
    expect(restored[0]?.pinned).toBe(true);
    expect(restored[1]?.accountId).toBe("w");
    expect(restored[0]?.id).not.toBe("a"); // fresh id
  });

  it("handles an empty layout", () => {
    const { tabs, activeTabId } = deserializeLayout({ tabs: [], activeIndex: 0 });
    expect(tabs).toHaveLength(0);
    expect(activeTabId).toBeNull();
  });
});
