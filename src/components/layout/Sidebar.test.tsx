// T101 — the TEAM nav badge reflects the pending-query count. The count hook is
// mocked so the badge renders deterministically regardless of backend state.
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";

import "@/i18n";

vi.mock("@/ipc/queries/queries", () => ({
  usePendingQueriesCount: () => ({ data: 3 }),
  pendingQueryKeys: { all: ["pendingQueries"], count: ["pendingQueries", "count"] },
}));

import Sidebar from "./Sidebar";

describe("Sidebar — TEAM badge (T101)", () => {
  it("shows the pending-query count on the Team nav item", () => {
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={qc}>
        <MemoryRouter>
          <Sidebar />
        </MemoryRouter>
      </QueryClientProvider>,
    );
    const badge = screen.getByLabelText(/3 questions awaiting your decision/);
    expect(badge).toHaveTextContent("3");
  });
});
