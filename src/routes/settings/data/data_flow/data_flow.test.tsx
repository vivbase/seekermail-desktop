// T054 + T069 acceptance: the five static data-flow entries render, the v0.4
// AI placeholder is gone, and the dynamic AI routing section discloses the
// real per-account endpoints (cloud vs on-device), the AI-off state, the
// fixed ADR-0004 no-proxy statement, and the 24h activity summary.
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";

import "@/i18n";
import * as client from "@/ipc/client";
import type { DataFlowAiRouting, AiRouteEntry } from "@/ipc/dataFlow";
import DataFlowPanel from "./index";

function routeEntry(overrides: Partial<AiRouteEntry>): AiRouteEntry {
  return {
    accountId: "acct-1",
    accountEmail: "legal@northwind.co",
    colorToken: "terra",
    aiProvider: "openai",
    aiModel: "gpt-4o",
    endpointKind: "cloud",
    endpointUrl: "https://api.openai.com/v1",
    endpointHost: "api.openai.com",
    isLocal: false,
    ...overrides,
  };
}

function payload(overrides: Partial<DataFlowAiRouting>): DataFlowAiRouting {
  return {
    routes: [],
    activity: [],
    sinceUnix: 1_780_000_000,
    ...overrides,
  };
}

function renderPage(routing: DataFlowAiRouting) {
  vi.spyOn(client, "ipc").mockResolvedValue(routing);
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <DataFlowPanel />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("DataFlowPanel — static rows (T054, AI row superseded by T069)", () => {
  it("renders the five static data-flow entries", () => {
    renderPage(payload({}));
    for (const label of [
      "Mail content",
      "Metadata",
      "Vector index",
      "IMAP / SMTP",
      "Update check",
    ]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
  });

  it("no longer shows the v0.4 no-AI placeholder notice", () => {
    renderPage(payload({}));
    expect(screen.queryByText(/No AI requests in v0\.4/)).not.toBeInTheDocument();
  });

  it("labels local flows with the Local badge", () => {
    renderPage(payload({}));
    // Mail content / metadata / vector index → three Local badges.
    expect(screen.getAllByText("Local").length).toBe(3);
  });
});

describe("AiRoutingSection — ADR-0004 no-proxy statement", () => {
  it("always renders the fixed no-proxy disclosure line", () => {
    renderPage(payload({}));
    expect(
      screen.getByText(
        "Your AI requests go directly to your chosen provider. SeekerMail servers are never in the path.",
      ),
    ).toBeInTheDocument();
  });
});

describe("AiRoutingSection — per-account routing rows", () => {
  it("shows a cloud account with the endpoint hostname, Cloud badge, and disclosure note", async () => {
    renderPage(payload({ routes: [routeEntry({})] }));
    await waitFor(() => expect(screen.getByText("legal@northwind.co")).toBeInTheDocument());

    expect(screen.getByText("api.openai.com")).toBeInTheDocument();
    expect(screen.getByText("Cloud")).toBeInTheDocument();
    expect(screen.getByText("OpenAI")).toBeInTheDocument();
    expect(
      screen.getByText("Mail content is sent to this endpoint when AI runs for this account."),
    ).toBeInTheDocument();
  });

  it("shows a local (Ollama) account as on-device with no disclosure note", async () => {
    renderPage(
      payload({
        routes: [
          routeEntry({
            accountId: "acct-2",
            accountEmail: "personal@example.com",
            colorToken: "sage",
            aiProvider: "ollama",
            aiModel: "llama3:8b",
            endpointKind: "local",
            endpointUrl: "http://localhost:11434",
            endpointHost: "localhost:11434",
            isLocal: true,
          }),
        ],
      }),
    );
    await waitFor(() => expect(screen.getByText("personal@example.com")).toBeInTheDocument());

    expect(screen.getByText("On this device")).toBeInTheDocument();
    expect(screen.getByText("On-device")).toBeInTheDocument();
    expect(
      screen.queryByText("Mail content is sent to this endpoint when AI runs for this account."),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("Cloud")).not.toBeInTheDocument();
  });

  it("shows an in-process (local ONNX) account as on-device", async () => {
    renderPage(
      payload({
        routes: [
          routeEntry({
            accountId: "acct-4",
            accountEmail: "archive@example.com",
            aiProvider: "local_onnx",
            aiModel: null,
            endpointKind: "in_process",
            endpointUrl: null,
            endpointHost: null,
            isLocal: true,
          }),
        ],
      }),
    );
    await waitFor(() =>
      expect(screen.getByText("In-process (on this device)")).toBeInTheDocument(),
    );
    expect(screen.getByText("On-device")).toBeInTheDocument();
  });

  it("shows an AI-off account as a muted disabled row", async () => {
    renderPage(
      payload({
        routes: [
          routeEntry({
            accountId: "acct-3",
            accountEmail: "backup@example.com",
            colorToken: "amber",
            aiProvider: "none",
            aiModel: null,
            endpointKind: "none",
            endpointUrl: null,
            endpointHost: null,
            isLocal: false,
          }),
        ],
      }),
    );
    await waitFor(() => expect(screen.getByText("backup@example.com")).toBeInTheDocument());

    expect(
      screen.getByText("AI disabled for this account — mail content stays on this device."),
    ).toBeInTheDocument();
    expect(screen.queryByText("Cloud")).not.toBeInTheDocument();
    expect(screen.queryByText("On-device")).not.toBeInTheDocument();
  });
});

describe("AiRoutingSection — 24h activity summary", () => {
  it("renders request and token totals per account/capability bucket", async () => {
    renderPage(
      payload({
        routes: [routeEntry({})],
        activity: [
          {
            accountId: "acct-1",
            decisionType: "draft_reply",
            aiModel: "openai/gpt-4o",
            requestCount: 3,
            inputTokens: 900,
            outputTokens: 100,
          },
        ],
      }),
    );
    await waitFor(() =>
      expect(screen.getByText("AI Activity — Last 24 Hours")).toBeInTheDocument(),
    );

    expect(screen.getByText("draft_reply")).toBeInTheDocument();
    expect(screen.getByText("3 requests")).toBeInTheDocument();
    // intl-messageformat may or may not apply digit grouping to `#`.
    expect(screen.getByText(/1,?000 tokens/)).toBeInTheDocument();
    expect(screen.queryByText("No AI requests in the last 24 hours.")).not.toBeInTheDocument();
  });

  it("shows the empty state when there was no AI activity", async () => {
    renderPage(payload({ routes: [routeEntry({})] }));
    await waitFor(() =>
      expect(screen.getByText("No AI requests in the last 24 hours.")).toBeInTheDocument(),
    );
  });
});
