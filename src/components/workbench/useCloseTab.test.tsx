import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { renderHook } from "@testing-library/react";

import "@/i18n";
import { useWorkbench } from "@/stores/workbench";
import { useCloseTabWithGuard } from "./useCloseTab";

describe("useCloseTabWithGuard (WB-06)", () => {
  beforeEach(() => {
    useWorkbench.setState({ tabs: [], activeTabId: null, recentlyClosed: [] });
  });
  afterEach(() => vi.restoreAllMocks());

  it("closes a clean tab without confirming", () => {
    const a = useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "x" });
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const { result } = renderHook(() => useCloseTabWithGuard());
    expect(result.current(a)).toBe(true);
    expect(confirm).not.toHaveBeenCalled();
    expect(useWorkbench.getState().tabs).toHaveLength(0);
  });

  it("confirms before closing a dirty tab; cancel keeps it", () => {
    const a = useWorkbench.getState().openTab({ route: { page: "compose" } });
    useWorkbench.getState().setDirty(a, true);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    const { result } = renderHook(() => useCloseTabWithGuard());
    expect(result.current(a)).toBe(false);
    expect(confirm).toHaveBeenCalledOnce();
    expect(useWorkbench.getState().tabs).toHaveLength(1); // kept
  });

  it("closes a dirty tab when confirmed", () => {
    const a = useWorkbench.getState().openTab({ route: { page: "compose" } });
    useWorkbench.getState().setDirty(a, true);
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const { result } = renderHook(() => useCloseTabWithGuard());
    expect(result.current(a)).toBe(true);
    expect(useWorkbench.getState().tabs).toHaveLength(0);
  });
});
