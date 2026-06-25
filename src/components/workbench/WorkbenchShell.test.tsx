import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import { createMemoryRouter, Link, RouterProvider, type RouteObject } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import "@/i18n";

// Render the shell without the full data stack: stub the account gate + heavy global children.
vi.mock("@/lib/accountGate", () => ({ useHasAccounts: () => true }));
vi.mock("@/components/layout/RiskBanner", () => ({ default: () => null }));
vi.mock("@/components/ai/AiActivationPrompt", () => ({ default: () => null }));

import { useWorkbench } from "@/stores/workbench";
import WorkbenchShell from "./WorkbenchShell";

// Stub per-tab routes (no Sidebar/WorkspaceLayout) — distinct text per page + an in-tab link.
const stubRoutes: RouteObject[] = [
  { path: "/", element: <div>PAGE-DASH</div> },
  { path: "/search", element: <div>PAGE-SEARCH</div> },
  {
    path: "/all-mail",
    element: (
      <div>
        <span>PAGE-INBOX</span>
        <Link to="/search">go-search</Link>
      </div>
    ),
  },
];

function renderShell() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <WorkbenchShell routes={stubRoutes} />
    </QueryClientProvider>,
  );
}

describe("WorkbenchShell (WB-09 v2 — independent per-tab routers)", () => {
  beforeEach(() => {
    useWorkbench.setState({ tabs: [], activeTabId: null, recentlyClosed: [] });
  });

  it("renders each tab as its own router; navigating one tab does NOT move the active tab", () => {
    const inbox = useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "x" }); // /all-mail
    useWorkbench.getState().openTab({ route: { page: "dashboard" } }); // /
    useWorkbench.getState().activateTab(inbox); // inbox tab active
    const { container } = renderShell();

    const panes = () =>
      Array.from(container.querySelectorAll("[data-tab-active]")) as HTMLElement[];
    expect(within(panes()[0]!).getByText("PAGE-INBOX")).toBeInTheDocument();
    expect(within(panes()[1]!).getByText("PAGE-DASH")).toBeInTheDocument();
    expect(panes()[0]!).toHaveAttribute("data-tab-active", "true");

    // Navigate INSIDE the inbox tab (its own router) → only that tab changes...
    fireEvent.click(within(panes()[0]!).getByText("go-search"));
    expect(within(panes()[0]!).getByText("PAGE-SEARCH")).toBeInTheDocument();
    // ...the other tab is untouched...
    expect(within(panes()[1]!).getByText("PAGE-DASH")).toBeInTheDocument();
    // ...and the active tab did NOT jump.
    expect(useWorkbench.getState().activeTabId).toBe(inbox);
  });

  it("mounts the tab strip above the tabs", () => {
    useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "x" });
    renderShell();
    expect(screen.getByRole("tablist")).toBeInTheDocument();
  });

  // Regression: the shell is mounted as a route element of App.tsx's data router
  // (RouterProvider). A bare per-tab <MemoryRouter> then nests inside the outer
  // router and React Router throws "cannot render a <Router> inside another
  // <Router>" — which renders as a blank "Unexpected Application Error!" on launch.
  // The per-tab routers must be context-isolated (IsolatedTabRouter) so this works.
  it("mounts inside the App data router (RouterProvider) without the nested-<Router> crash", () => {
    useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "x" }); // /all-mail
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const router = createMemoryRouter(
      [
        {
          element: <WorkbenchShell routes={stubRoutes} />,
          children: [{ index: true, element: <div>outlet-unused</div> }],
        },
        { path: "/onboarding", element: <div>Onboarding</div> },
      ],
      { initialEntries: ["/"] },
    );
    render(
      <QueryClientProvider client={qc}>
        <RouterProvider router={router} />
      </QueryClientProvider>,
    );
    expect(screen.queryByText(/Unexpected Application Error/i)).not.toBeInTheDocument();
    expect(screen.getByText("PAGE-INBOX")).toBeInTheDocument();
  });
});
