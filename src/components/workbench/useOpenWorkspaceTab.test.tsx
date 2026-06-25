import { describe, it, expect, beforeEach } from "vitest";
import { renderHook } from "@testing-library/react";

import "@/i18n";
import { useWorkbench } from "@/stores/workbench";
import { useToastStore } from "@/components/ui/Toast";
import { useOpenWorkspaceTab } from "./useOpenWorkspaceTab";

describe("useOpenWorkspaceTab (WB-05)", () => {
  beforeEach(() => {
    useWorkbench.setState({ tabs: [], activeTabId: null, recentlyClosed: [] });
    useToastStore.setState({ toasts: [] });
  });

  it("focuses an existing singleton and toasts (not on first open)", () => {
    const { result } = renderHook(() => useOpenWorkspaceTab());
    const first = result.current({ route: { page: "dashboard" } });
    expect(useToastStore.getState().toasts).toHaveLength(0); // fresh open → no toast

    const second = result.current({ route: { page: "dashboard" } });
    expect(second).toBe(first); // focused the same tab, no duplicate
    expect(useWorkbench.getState().tabs).toHaveLength(1);
    expect(useToastStore.getState().toasts).toHaveLength(1); // toast explains the focus
  });

  it("does not toast or dedupe non-singleton pages", () => {
    const { result } = renderHook(() => useOpenWorkspaceTab());
    result.current({ route: { page: "inbox" }, accountId: "a" });
    result.current({ route: { page: "inbox" }, accountId: "b" });
    expect(useWorkbench.getState().tabs).toHaveLength(2);
    expect(useToastStore.getState().toasts).toHaveLength(0);
  });
});
