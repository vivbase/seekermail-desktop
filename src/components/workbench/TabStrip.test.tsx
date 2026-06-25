import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent, within, act } from "@testing-library/react";

import "@/i18n";
import { useWorkbench } from "@/stores/workbench";
import TabStrip from "./TabStrip";

/** Open two tabs and return their ids. Order = [inbox, search]; search is active (opened last). */
function seedTwoTabs() {
  const a = useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "x" });
  const b = useWorkbench.getState().openTab({ route: { page: "search" } });
  return { a, b };
}

describe("TabStrip (WB-03 / WB-04)", () => {
  beforeEach(() => {
    useWorkbench.setState({ tabs: [], activeTabId: null, recentlyClosed: [] });
  });

  it("renders a tab per store tab inside a tablist", () => {
    seedTwoTabs();
    render(<TabStrip />);
    expect(screen.getByRole("tablist")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Inbox" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Search" })).toBeInTheDocument();
  });

  it("activates a tab on click", () => {
    const { a } = seedTwoTabs();
    render(<TabStrip />);
    // Search is active initially; click Inbox to activate it.
    fireEvent.click(screen.getByRole("tab", { name: "Inbox" }));
    expect(useWorkbench.getState().activeTabId).toBe(a);
    expect(screen.getByRole("tab", { name: "Inbox" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "Search" })).toHaveAttribute("aria-selected", "false");
  });

  it("opens a new tab from the + button", () => {
    seedTwoTabs();
    render(<TabStrip />);
    fireEvent.click(screen.getByRole("button", { name: "New tab" }));
    expect(useWorkbench.getState().tabs).toHaveLength(3);
  });

  it("closes a tab and moves focus to a neighbour", () => {
    const { a } = seedTwoTabs();
    render(<TabStrip />);
    act(() => useWorkbench.getState().activateTab(a)); // make Inbox active
    const inbox = screen.getByRole("tab", { name: "Inbox" });
    fireEvent.click(within(inbox).getByRole("button", { name: "Close tab" }));

    expect(screen.queryByRole("tab", { name: "Inbox" })).not.toBeInTheDocument();
    // Focus moved to the neighbour (Search), which is now active.
    expect(screen.getByRole("tab", { name: "Search" })).toHaveFocus();
    expect(useWorkbench.getState().tabs).toHaveLength(1);
  });

  it("reorders tabs by drag within the strip", () => {
    const { a, b } = seedTwoTabs();
    render(<TabStrip />);
    const inbox = screen.getByRole("tab", { name: "Inbox" });
    const search = screen.getByRole("tab", { name: "Search" });
    fireEvent.dragStart(inbox);
    fireEvent.dragOver(search);
    fireEvent.drop(search);
    expect(useWorkbench.getState().tabs.map((t) => t.id)).toEqual([b, a]);
  });

  it("colours the account dot via accentFor", () => {
    seedTwoTabs();
    render(<TabStrip accentFor={(id) => (id === "x" ? "terra" : undefined)} />);
    const inbox = screen.getByRole("tab", { name: "Inbox" });
    expect(inbox.querySelector(".bg-terra")).not.toBeNull();
  });

  it("shows an unsaved indicator on dirty tabs", () => {
    const { a } = seedTwoTabs();
    useWorkbench.getState().setDirty(a, true);
    render(<TabStrip />);
    const inbox = screen.getByRole("tab", { name: "Inbox" });
    expect(inbox.querySelector("[data-dirty-dot]")).not.toBeNull();
  });

  it("pins a tab via the context menu and renders it leftmost (WB-07)", () => {
    seedTwoTabs(); // [Inbox, Search]; Search active
    render(<TabStrip />);
    fireEvent.contextMenu(screen.getByRole("tab", { name: "Search" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Pin tab" }));
    expect(useWorkbench.getState().tabs.find((tb) => tb.route.page === "search")?.pinned).toBe(
      true,
    );
    // pinned tab now renders leftmost
    expect(screen.getAllByRole("tab")[0]).toHaveAttribute("aria-label", "Search");
  });
});
