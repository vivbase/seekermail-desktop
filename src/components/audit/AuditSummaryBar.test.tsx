// T089 — AuditSummaryBar: four stat values from get_ai_decisions_summary and
// the pulse skeleton while loading.
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { DecisionSummary } from "@shared/bindings";

import "@/i18n";
import * as client from "@/ipc/client";
import { AuditSummaryBar } from "./AuditSummaryBar";

const SUMMARY: DecisionSummary = {
  totalEvents: 128,
  autoSentCount: 42,
  downgradeCount: 1,
  sensitiveCount: 3,
  draftSentCount: 60,
  draftCreatedCount: 70,
  totalInputTokens: 90_000,
  totalOutputTokens: 10_000,
  successRate: 0.97,
};

function renderWithClient(ui: React.ReactElement) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>);
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("AuditSummaryBar", () => {
  it("renders the four summary values", async () => {
    const spy = vi.spyOn(client, "ipc").mockResolvedValue(SUMMARY);
    renderWithClient(<AuditSummaryBar accountId={null} sinceUnix={0} untilUnix={1000} />);

    await waitFor(() => expect(screen.getByText("128")).toBeInTheDocument());
    expect(screen.getByText("42")).toBeInTheDocument();
    expect(screen.getByText("97%")).toBeInTheDocument();
    expect(screen.getByText((100_000).toLocaleString())).toBeInTheDocument();
    expect(spy).toHaveBeenCalledWith("get_ai_decisions_summary", {
      accountId: null,
      sinceUnix: 0,
      untilUnix: 1000,
    });
    // Labels use the uppercase --fu section-label treatment.
    expect(screen.getByText("Total Events")).toBeInTheDocument();
  });

  it("shows skeleton blocks (and no numbers) while loading", () => {
    vi.spyOn(client, "ipc").mockReturnValue(new Promise(() => {})); // never resolves
    renderWithClient(<AuditSummaryBar accountId={null} sinceUnix={0} untilUnix={1000} />);

    expect(screen.getAllByTestId("summary-skeleton")).toHaveLength(4);
    expect(screen.queryByText("128")).not.toBeInTheDocument();
  });
});
