import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, act, waitFor } from "@testing-library/react";
import { MemoryRouter, useLocation } from "react-router-dom";

import "@/i18n";
import { useWorkbench } from "@/stores/workbench";
import WorkbenchRoot, { type WorkspacePages } from "./WorkbenchRoot";

function LocationProbe() {
  const loc = useLocation();
  return <span data-testid="loc">{loc.pathname}</span>;
}

const pages: WorkspacePages = {
  dashboard: () => <div>DashPage</div>,
  inbox: ({ tab }) => <div>InboxPage {tab.accountId ?? "all"}</div>,
  search: () => <div>SearchPage</div>,
};

function renderRoot() {
  return render(
    <MemoryRouter initialEntries={["/"]}>
      <WorkbenchRoot pages={pages} />
      <LocationProbe />
    </MemoryRouter>,
  );
}

describe("WorkbenchRoot (WB-09 mechanism)", () => {
  beforeEach(() => {
    useWorkbench.setState({ tabs: [], activeTabId: null, recentlyClosed: [] });
  });

  it("renders the active tab's page and keeps the others mounted (hidden)", () => {
    useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "x" });
    useWorkbench.getState().openTab({ route: { page: "search" } }); // active = search
    renderRoot();

    const searchPane = screen.getByText("SearchPage").closest("[data-tab-active]");
    const inboxPane = screen.getByText(/InboxPage/).closest("[data-tab-active]");
    expect(searchPane).toHaveAttribute("data-tab-active", "true");
    expect(inboxPane).toHaveAttribute("data-tab-active", "false");
  });

  it("mirrors the active tab's route to the URL", async () => {
    const a = useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "x" });
    useWorkbench.getState().openTab({ route: { page: "search" } }); // active = search
    renderRoot();

    await waitFor(() => expect(screen.getByTestId("loc")).toHaveTextContent(/^\/search$/));

    act(() => useWorkbench.getState().activateTab(a));
    await waitFor(() => expect(screen.getByTestId("loc")).toHaveTextContent(/^\/all-mail$/));
  });

  it("navigating within the active tab swaps the page and the URL", async () => {
    const a = useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "x" });
    renderRoot();
    await waitFor(() => expect(screen.getByTestId("loc")).toHaveTextContent(/^\/all-mail$/));

    act(() => useWorkbench.getState().navigateTab(a, { page: "dashboard" }));
    await waitFor(() => expect(screen.getByTestId("loc")).toHaveTextContent(/^\/$/));
    const dashPane = screen.getByText("DashPage").closest("[data-tab-active]");
    expect(dashPane).toHaveAttribute("data-tab-active", "true");
  });

  it("renders the risk layer above the tab strip (WB-10)", () => {
    useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "x" });
    render(
      <MemoryRouter initialEntries={["/"]}>
        <WorkbenchRoot pages={pages} riskLayer={<div data-testid="risk">RISK</div>} />
      </MemoryRouter>,
    );
    const risk = screen.getByTestId("risk");
    const tablist = screen.getByRole("tablist");
    // risk precedes the tablist in document order → it is above the tabs (load-bearing).
    expect(risk.compareDocumentPosition(tablist) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});
