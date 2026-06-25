import { describe, it, expect, beforeEach, vi } from "vitest";
import { renderHook, fireEvent } from "@testing-library/react";

import { useWorkbench } from "@/stores/workbench";
import { useWorkbenchShortcuts } from "./useWorkbenchShortcuts";

function press(init: KeyboardEventInit & { key: string }) {
  fireEvent.keyDown(window, init);
}

describe("useWorkbenchShortcuts (WB-08)", () => {
  beforeEach(() => {
    useWorkbench.setState({ tabs: [], activeTabId: null, recentlyClosed: [] });
  });

  it("Ctrl+T opens a new tab", () => {
    renderHook(() => useWorkbenchShortcuts());
    press({ key: "t", ctrlKey: true });
    expect(useWorkbench.getState().tabs).toHaveLength(1);
  });

  it("Ctrl+W closes the active tab", () => {
    const a = useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "x" });
    renderHook(() => useWorkbenchShortcuts());
    press({ key: "w", ctrlKey: true });
    expect(useWorkbench.getState().tabs.find((t) => t.id === a)).toBeUndefined();
  });

  it("Ctrl+Shift+T reopens the last-closed tab", () => {
    const a = useWorkbench.getState().openTab({ route: { page: "search" } });
    useWorkbench.getState().closeTab(a);
    renderHook(() => useWorkbenchShortcuts());
    press({ key: "T", ctrlKey: true, shiftKey: true });
    expect(useWorkbench.getState().tabs).toHaveLength(1);
    expect(useWorkbench.getState().tabs[0]?.route.page).toBe("search");
  });

  it("Ctrl+Tab cycles to the next tab (wrapping)", () => {
    const a = useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "x" });
    const b = useWorkbench.getState().openTab({ route: { page: "search" } }); // active = b
    renderHook(() => useWorkbenchShortcuts());
    press({ key: "Tab", ctrlKey: true }); // b → wraps to a
    expect(useWorkbench.getState().activeTabId).toBe(a);
    press({ key: "Tab", ctrlKey: true }); // a → b
    expect(useWorkbench.getState().activeTabId).toBe(b);
  });

  it("Ctrl+P invokes the switcher callback", () => {
    const onOpenSwitcher = vi.fn();
    renderHook(() => useWorkbenchShortcuts({ onOpenSwitcher }));
    press({ key: "p", ctrlKey: true });
    expect(onOpenSwitcher).toHaveBeenCalledOnce();
  });
});
