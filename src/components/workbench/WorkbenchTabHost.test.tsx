import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, act, fireEvent } from "@testing-library/react";
import { useEffect } from "react";

import { useWorkbench } from "@/stores/workbench";
import WorkbenchTabHost from "./WorkbenchTabHost";
import { useIsTabActive } from "./tabActiveContext";

// A child that records mounts and holds uncontrolled input state, so we can prove a tab is
// never remounted (keep-alive) and its DOM state survives switches.
let mountCount = 0;
function Probe({ tag }: { tag: string }) {
  const active = useIsTabActive();
  useEffect(() => {
    mountCount += 1;
  }, []);
  return (
    <div>
      <span data-testid={`active-${tag}`}>{active ? "active" : "inactive"}</span>
      <input data-testid={`input-${tag}`} defaultValue="" />
    </div>
  );
}

function renderHost() {
  return render(<WorkbenchTabHost renderWorkspace={(tab) => <Probe tag={tab.id} />} />);
}

describe("WorkbenchTabHost (WB-02)", () => {
  beforeEach(() => {
    useWorkbench.setState({ tabs: [], activeTabId: null, recentlyClosed: [] });
    mountCount = 0;
  });

  it("renders one pane per tab; only the active pane is visible", () => {
    const a = useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "x" });
    const b = useWorkbench.getState().openTab({ route: { page: "search" } });
    renderHost();

    expect(screen.getByTestId(`active-${a}`)).toBeInTheDocument();
    expect(screen.getByTestId(`active-${b}`)).toBeInTheDocument();

    // b was opened last → active; a is kept alive but hidden.
    expect(screen.getByTestId(`active-${b}`)).toHaveTextContent("active");
    expect(screen.getByTestId(`active-${a}`)).toHaveTextContent("inactive");

    const paneA = screen.getByTestId(`input-${a}`).closest("[data-tab-active]");
    expect(paneA).toHaveAttribute("data-tab-active", "false");
    expect((paneA as HTMLElement).style.display).toBe("none");
  });

  it("keeps inactive tabs mounted — input state survives a tab switch (keep-alive)", () => {
    const a = useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "x" });
    const b = useWorkbench.getState().openTab({ route: { page: "search" } });
    renderHost();
    const mountsAfterRender = mountCount; // both panes mounted exactly once

    act(() => useWorkbench.getState().activateTab(a));
    fireEvent.change(screen.getByTestId(`input-${a}`), { target: { value: "draft text" } });
    act(() => useWorkbench.getState().activateTab(b));
    act(() => useWorkbench.getState().activateTab(a));

    // Value persisted (the pane was never unmounted) ...
    expect((screen.getByTestId(`input-${a}`) as HTMLInputElement).value).toBe("draft text");
    // ... and no pane was remounted across the switches.
    expect(mountCount).toBe(mountsAfterRender);
  });

  it("exposes per-tab active state via useIsTabActive", () => {
    const a = useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "x" });
    const b = useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "y" });
    renderHost();

    expect(screen.getByTestId(`active-${b}`)).toHaveTextContent("active");

    act(() => useWorkbench.getState().activateTab(a));
    expect(screen.getByTestId(`active-${a}`)).toHaveTextContent("active");
    expect(screen.getByTestId(`active-${b}`)).toHaveTextContent("inactive");
  });
});
