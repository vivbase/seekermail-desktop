// T101 — the TEAM nav badge reflects unread agent messages + unresolved decision
// cards; AGENTS no longer carries a badge. The count hook is mocked so the badge
// renders deterministically regardless of backend state.
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";

import "@/i18n";

vi.mock("@/ipc/queries/im", () => ({
  useTeamUnreadCount: () => ({ data: 3 }),
}));

import Sidebar from "./Sidebar";

describe("Sidebar — TEAM badge (T101)", () => {
  it("shows the unread/attention count on Team and no badge on Agents", () => {
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const { container } = render(
      <QueryClientProvider client={qc}>
        <MemoryRouter>
          <Sidebar />
        </MemoryRouter>
      </QueryClientProvider>,
    );
    const badge = screen.getByLabelText(/3 Team items need your attention/);
    expect(badge).toHaveTextContent("3");
    // AGENTS dropped its badge, so the Team badge is the only nav badge.
    expect(container.querySelectorAll("span.bg-red.font-mono")).toHaveLength(1);
  });
});
