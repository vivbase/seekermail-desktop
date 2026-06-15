import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createMemoryRouter, RouterProvider } from "react-router-dom";

import "@/i18n";
import AppShell from "./AppShell";

function renderShell() {
  const router = createMemoryRouter(
    [
      {
        element: <AppShell />,
        children: [{ index: true, element: <div>Dashboard content</div> }],
      },
      { path: "/onboarding", element: <div>Onboarding</div> },
    ],
    { initialEntries: ["/"] },
  );
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
}

describe("AppShell", () => {
  it("renders the sidebar nav and routed content", async () => {
    renderShell();
    // Sidebar nav (left region)
    expect(screen.getByRole("link", { name: /dashboard/i })).toBeInTheDocument();
    // Main region routed content
    expect(await screen.findByText("Dashboard content")).toBeInTheDocument();
  });

  it("has no global command bar or right agent rail (prototype shell)", () => {
    renderShell();
    // The prototype shell removed the top command bar and the right agent rail.
    expect(screen.queryByText("No agents configured yet.")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Toggle agent panel")).not.toBeInTheDocument();
  });
});
