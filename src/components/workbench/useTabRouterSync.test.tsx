import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, act, waitFor, fireEvent } from "@testing-library/react";
import { MemoryRouter, useLocation, useNavigate } from "react-router-dom";

import { useWorkbench } from "@/stores/workbench";
import { useTabRouterSync } from "./useTabRouterSync";

function Harness() {
  useTabRouterSync();
  const loc = useLocation();
  const navigate = useNavigate();
  return (
    <div>
      <span data-testid="loc">{loc.pathname}</span>
      <button onClick={() => navigate("/search")}>go-search</button>
    </div>
  );
}

const render0 = () =>
  render(
    <MemoryRouter initialEntries={["/"]}>
      <Harness />
    </MemoryRouter>,
  );

describe("useTabRouterSync (WB-09 live glue)", () => {
  beforeEach(() => {
    useWorkbench.setState({ tabs: [], activeTabId: null, recentlyClosed: [] });
  });

  it("opens + activates a tab for the initial route", async () => {
    render0();
    await waitFor(() => {
      const s = useWorkbench.getState();
      expect(s.tabs).toHaveLength(1);
      expect(s.tabs[0]?.route.page).toBe("dashboard");
      expect(s.activeTabId).toBe(s.tabs[0]?.id);
    });
  });

  it("navigating the router opens a tab for the new route", async () => {
    render0();
    await waitFor(() => expect(useWorkbench.getState().tabs).toHaveLength(1));
    fireEvent.click(screen.getByText("go-search"));
    await waitFor(() => {
      const pages = useWorkbench.getState().tabs.map((t) => t.route.page);
      expect(pages).toContain("search");
      expect(screen.getByTestId("loc")).toHaveTextContent("/search");
    });
  });

  it("activating a tab navigates the router to its route (no loop)", async () => {
    render0();
    await waitFor(() => expect(useWorkbench.getState().tabs).toHaveLength(1));
    const dash = useWorkbench.getState().tabs[0]!.id;
    // open a second (search) tab via the store, then switch back to dashboard
    fireEvent.click(screen.getByText("go-search"));
    await waitFor(() => expect(screen.getByTestId("loc")).toHaveTextContent("/search"));
    act(() => useWorkbench.getState().activateTab(dash));
    await waitFor(() => expect(screen.getByTestId("loc")).toHaveTextContent(/^\/$/));
  });
});
