// T071 tests — D1 Legal sidebar (sorting, click-to-highlight, retry, provider
// gap), the T4 non-dismissable RiskAlertBanner, the post-DOMPurify body
// highlight injection, and the report risk panel's T4 dismiss suppression.
// Off-Tauri, `ipc()` resolves from the mock layer in client.ts unless spied.
import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import React from "react";
import type { MailDetail } from "@shared/bindings";

import "@/i18n";
import * as client from "@/ipc/client";
import type { LegalAnalysisResult, RiskEvent } from "@/ipc/legal";
import { useSelection } from "@/stores/selection";
import { LegalSidebar } from "./LegalSidebar";
import { RiskAlertBanner } from "./RiskAlertBanner";
import { MailBody } from "./MailBody";
import { injectHighlight } from "./SanitizedMail";
import { RiskEventsPanel } from "@/routes/report/RiskEventsPanel";

const ANALYSIS: LegalAnalysisResult = {
  decisionId: "dec-1",
  mailId: "m-1",
  accountId: "demo-1",
  riskList: [
    {
      level: "low",
      type: "other",
      originalText: "attachment provenance",
      finding: "Attachment provenance is not verified",
      suggestion: "Confirm the attachment hash with the sender",
    },
    {
      level: "high",
      type: "liability",
      originalText: "lock the board deck",
      finding: "Figures become binding once presented",
      suggestion: "Add a draft watermark until audited",
    },
    {
      level: "medium",
      type: "payment",
      originalText: "before Friday",
      finding: "Deadline set without a written change order",
      suggestion: "Request a signed change order first",
    },
  ],
  keyClauses: {
    payment: "Net 90",
    delivery: null,
    liability: null,
    confidentiality: null,
    disputeResolution: null,
  },
  complianceAdvice: ["Route through the finance approval workflow."],
  overallLevel: "high",
  aiModel: "mock-model",
  knowledgeRefs: [],
  createdAt: 1,
};

const T4_EVENT: RiskEvent = {
  id: "risk-t4",
  mailId: "m-1",
  accountId: "demo-1",
  riskLevel: 4,
  riskType: "payment_anomaly",
  evidence: {},
  description: "Unusually long payment term conflicts with the net-30 policy.",
  status: "open",
  expiresAt: null,
  createdAt: 1,
};

const T3_EVENT: RiskEvent = {
  ...T4_EVENT,
  id: "risk-t3",
  mailId: "m-2",
  riskLevel: 3,
  riskType: "rule_conflict",
  description: "Liability clause exceeds the negotiated cap.",
  expiresAt: 9999999999,
};

function newQueryClient() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
}

/** Index into a queried element array, failing loudly (noUncheckedIndexedAccess). */
function nth<T>(arr: T[], i: number): T {
  const v = arr[i];
  if (v === undefined) throw new Error(`expected element at index ${i}`);
  return v;
}

function renderWithProviders(ui: React.ReactElement) {
  const qc = newQueryClient();
  render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>{ui}</MemoryRouter>
    </QueryClientProvider>,
  );
  return qc;
}

beforeEach(() => {
  useSelection.getState().setLegalHighlight(null);
});

afterEach(() => {
  vi.restoreAllMocks();
});

// ── LegalSidebar ──────────────────────────────────────────────────────────────

describe("LegalSidebar — analysis render path", () => {
  it("runs the mutation on demand and renders risks sorted high → low", async () => {
    const ipcSpy = vi.spyOn(client, "ipc").mockResolvedValue(ANALYSIS);
    renderWithProviders(<LegalSidebar mailId="m-1" />);

    // Trigger state first; nothing fetched before the user asks.
    expect(ipcSpy).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: /Analyze Legal Risk/ }));

    await waitFor(() => expect(screen.getByText("High Risk")).toBeInTheDocument());
    expect(ipcSpy).toHaveBeenCalledWith("analyze_legal_risk", {
      params: { mailId: "m-1", forceNew: false },
    });

    // Sorted by severity: liability (high), payment (medium), other (low).
    const risksRegion = screen.getByRole("region", { name: "Risks" });
    const items = within(risksRegion).getAllByRole("listitem");
    expect(items[0]).toHaveTextContent("Liability");
    expect(items[1]).toHaveTextContent("Payment");
    expect(items[2]).toHaveTextContent("Other");

    // Key clauses + advice + resident disclaimer all present.
    expect(screen.getByText("Net 90")).toBeInTheDocument();
    expect(screen.getByText("Route through the finance approval workflow.")).toBeInTheDocument();
    expect(screen.getByText("AI initial assessment only — not legal advice.")).toBeInTheDocument();
  });

  it("clicking a risk's excerpt sets (and toggles) the legal highlight", async () => {
    vi.spyOn(client, "ipc").mockResolvedValue(ANALYSIS);
    renderWithProviders(<LegalSidebar mailId="m-1" />);
    fireEvent.click(screen.getByRole("button", { name: /Analyze Legal Risk/ }));
    await waitFor(() => expect(screen.getByText("High Risk")).toBeInTheDocument());

    // Expand the top (high) risk, then click its excerpt.
    fireEvent.click(nth(screen.getAllByRole("button", { expanded: false }), 0));
    const excerpt = screen.getByRole("button", {
      name: "Highlight in message",
    });
    fireEvent.click(excerpt);
    expect(useSelection.getState().legalHighlightText).toBe("lock the board deck");
    expect(excerpt).toHaveAttribute("aria-pressed", "true");

    // Clicking again clears the highlight.
    fireEvent.click(excerpt);
    expect(useSelection.getState().legalHighlightText).toBeNull();
  });

  it("shows the failure state and retries with forceNew: true", async () => {
    const ipcSpy = vi
      .spyOn(client, "ipc")
      .mockRejectedValueOnce({
        code: "INTERNAL",
        message: "model output invalid",
        detail: null,
      })
      .mockResolvedValue(ANALYSIS);
    renderWithProviders(<LegalSidebar mailId="m-1" />);

    fireEvent.click(screen.getByRole("button", { name: /Analyze Legal Risk/ }));
    await waitFor(() =>
      expect(screen.getByText("Analysis failed. Try again.")).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByRole("button", { name: "Try Again" }));
    await waitFor(() => expect(screen.getByText("High Risk")).toBeInTheDocument());
    expect(ipcSpy).toHaveBeenLastCalledWith("analyze_legal_risk", {
      params: { mailId: "m-1", forceNew: true },
    });
  });

  it("degrades to the provider hint with an /agents link on AI_PROVIDER_UNREACHABLE", async () => {
    vi.spyOn(client, "ipc").mockRejectedValue({
      code: "AI_PROVIDER_UNREACHABLE",
      message: "no provider",
      detail: null,
    });
    renderWithProviders(<LegalSidebar mailId="m-1" />);

    fireEvent.click(screen.getByRole("button", { name: /Analyze Legal Risk/ }));
    await waitFor(() => expect(screen.getByText("No AI provider configured.")).toBeInTheDocument());
    expect(screen.getByRole("link", { name: "Go to Settings → Agents" })).toHaveAttribute(
      "href",
      "/agents",
    );
  });
});

// ── RiskAlertBanner (T4 — non-dismissable) ────────────────────────────────────

describe("RiskAlertBanner", () => {
  it("renders as role=alert with NO close/dismiss affordance", () => {
    renderWithProviders(<RiskAlertBanner event={T4_EVENT} />);

    const banner = screen.getByRole("alert");
    expect(banner).toHaveTextContent("High-Risk Alert");
    expect(banner).toHaveTextContent(T4_EVENT.description);

    // The ONLY button is "Mark Resolved" — never a close/dismiss control.
    const buttons = within(banner).getAllByRole("button");
    expect(buttons).toHaveLength(1);
    expect(buttons[0]).toHaveTextContent("Mark Resolved");
    expect(within(banner).queryByRole("button", { name: /close/i })).toBeNull();
    expect(within(banner).queryByRole("button", { name: /dismiss/i })).toBeNull();
  });

  it("'Mark Resolved' calls resolve_risk_event with status resolved", async () => {
    const ipcSpy = vi.spyOn(client, "ipc").mockResolvedValue(null);
    renderWithProviders(<RiskAlertBanner event={T4_EVENT} />);

    fireEvent.click(screen.getByRole("button", { name: "Mark Resolved" }));
    await waitFor(() =>
      expect(ipcSpy).toHaveBeenCalledWith("resolve_risk_event", {
        params: { id: "risk-t4", status: "resolved" },
      }),
    );
  });
});

// ── Body highlight injection ──────────────────────────────────────────────────

const HIGHLIGHT_MAIL: MailDetail = {
  id: "m-1",
  accountId: "demo-1",
  threadId: "t-1",
  subject: "Q4 budget review",
  fromName: "Alice",
  fromEmail: "alice@northwind.co",
  to: [],
  cc: [],
  dateSent: 1,
  bodyHtml: "<p>Please lock the board deck before Friday.</p>",
  bodyText: "Please lock the board deck before Friday.",
  isRead: true,
  isStarred: false,
  isArchived: false,
  hasAttachments: false,
  folder: "INBOX",
};

describe("MailBody — legal highlight", () => {
  it("wraps the stored excerpt in <mark class='legal-highlight'>", () => {
    useSelection.getState().setLegalHighlight("lock the board deck");
    const { container } = render(<MailBody mail={HIGHLIGHT_MAIL} />);
    const mark = container.querySelector("mark.legal-highlight");
    expect(mark).not.toBeNull();
    expect(mark).toHaveTextContent("lock the board deck");
  });

  it("renders no mark when the highlight is cleared", () => {
    useSelection.getState().setLegalHighlight(null);
    const { container } = render(<MailBody mail={HIGHLIGHT_MAIL} />);
    expect(container.querySelector("mark.legal-highlight")).toBeNull();
  });
});

describe("injectHighlight — XSS safety", () => {
  it("wraps only the first occurrence", () => {
    const out = injectHighlight("<p>net 90 and net 90</p>", "net 90");
    expect(out).toBe('<p><mark class="legal-highlight">net 90</mark> and net 90</p>');
  });

  it("refuses phrases containing markup so script can never be injected", () => {
    const html = "<p>safe content</p>";
    expect(injectHighlight(html, '<script>alert("x")</script>')).toBe(html);
    expect(injectHighlight(html, "<img src=x onerror=alert(1)>")).toBe(html);
  });

  it("treats regex metacharacters in the phrase as literals", () => {
    const out = injectHighlight("<p>pay $100 (net 30)</p>", "$100 (net 30)");
    expect(out).toContain('<mark class="legal-highlight">$100 (net 30)</mark>');
  });
});

// ── RiskEventsPanel (/report) ─────────────────────────────────────────────────

describe("RiskEventsPanel", () => {
  it("offers Dismiss for non-T4 events but ONLY Resolve for T4", async () => {
    vi.spyOn(client, "ipc").mockResolvedValue([T4_EVENT, T3_EVENT]);
    renderWithProviders(<RiskEventsPanel />);

    await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(2));
    const rows = screen.getAllByRole("listitem");
    const t4Row = nth(rows, 0);
    const t3Row = nth(rows, 1);

    expect(within(t4Row).getByRole("button", { name: "Mark Resolved" })).toBeInTheDocument();
    expect(within(t4Row).queryByRole("button", { name: "Dismiss" })).toBeNull();

    expect(within(t3Row).getByRole("button", { name: "Mark Resolved" })).toBeInTheDocument();
    expect(within(t3Row).getByRole("button", { name: "Dismiss" })).toBeInTheDocument();
  });

  it("shows the empty state when no open events exist", async () => {
    vi.spyOn(client, "ipc").mockResolvedValue([]);
    renderWithProviders(<RiskEventsPanel />);
    await waitFor(() => expect(screen.getByText("No open risk events.")).toBeInTheDocument());
  });
});
