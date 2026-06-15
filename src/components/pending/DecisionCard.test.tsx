// T099 — DecisionCard: chips, submit → answer_query, skip → confirm → skip_query,
// and the T4 view-original-email affordance. Off-Tauri `ipc()` resolves from the
// stateful mock layer; here we spy to assert the command calls.
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

import "@/i18n";
import * as client from "@/ipc/client";
import type { PendingQuery, QaCardContent } from "@/ipc/pendingQueries";
import { DecisionCard } from "./DecisionCard";

function card(over: Partial<QaCardContent>): string {
  return JSON.stringify({
    cardVersion: 1,
    linkedQueryId: "pq-1",
    triggerType: "T1",
    priority: "normal",
    linkedEmailId: "m-2",
    questionText: "Do you recognise this sender?",
    options: [
      { id: "opt_known", label: "Yes, I know them", value: "known" },
      { id: "opt_unknown", label: "No, treat as unknown", value: "unknown" },
      { id: "opt_skip", label: "Skip", value: "__skip__" },
    ],
    multiSelect: false,
    freeTextPlaceholder: null,
    subQuestions: [],
    response: null,
    ...over,
  });
}

function query(over: Partial<PendingQuery> = {}): PendingQuery {
  return {
    id: "pq-1",
    accountId: "demo-1",
    mailId: "m-2",
    riskEventId: null,
    triggerType: "T1",
    question: "Do you recognise this sender?",
    options: card({}),
    answer: null,
    status: "pending",
    priority: 3,
    expiresAt: null,
    answeredAt: null,
    createdAt: 0,
    ...over,
  };
}

function withProviders(ui: ReactNode) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>{ui}</MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("DecisionCard", () => {
  it("renders answer chips (excluding Skip) and toggles selection", () => {
    const { container } = withProviders(<DecisionCard query={query()} />);
    expect(container.querySelector('[data-type="decision"]')).not.toBeNull();
    const chip = screen.getByRole("button", { name: "Yes, I know them" });
    expect(chip).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(chip);
    expect(chip).toHaveAttribute("aria-pressed", "true");
  });

  it("submits the selected answer via answer_query", async () => {
    const spy = vi.spyOn(client, "ipc");
    withProviders(<DecisionCard query={query()} />);
    fireEvent.click(screen.getByRole("button", { name: "No, treat as unknown" }));
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));
    await waitFor(() => expect(spy.mock.calls.some((c) => c[0] === "answer_query")).toBe(true));
  });

  it("skips via the confirmation dialog → skip_query", async () => {
    const spy = vi.spyOn(client, "ipc");
    withProviders(<DecisionCard query={query()} />);
    fireEvent.click(screen.getByRole("button", { name: "Skip" }));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Skip query" }));
    await waitFor(() => expect(spy.mock.calls.some((c) => c[0] === "skip_query")).toBe(true));
  });

  it("offers the view-original-email affordance for a T4 card", () => {
    const t4 = query({
      id: "pq-2",
      triggerType: "T4",
      options: card({
        triggerType: "T4",
        priority: "high",
        options: [
          { id: "opt_confirm", label: "Confirm and proceed", value: "confirm" },
          { id: "opt_block", label: "Block this email", value: "block" },
          { id: "opt_skip", label: "Skip", value: "__skip__" },
          { id: "opt_view_email", label: "View original email", value: "__view_email__" },
        ],
      }),
    });
    withProviders(<DecisionCard query={t4} />);
    // The view-email option renders as an open-email affordance, not a plain chip.
    expect(screen.getAllByRole("button", { name: /Open original email/i }).length).toBeGreaterThan(
      0,
    );
    expect(screen.getByRole("button", { name: "Confirm and proceed" })).toBeInTheDocument();
  });
});
