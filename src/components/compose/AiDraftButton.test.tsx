import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import React from "react";

import "@/i18n";
import { useCompose } from "@/stores/compose";
import { AiDraftButton } from "./AiDraftButton";

// Mock the generation hook so no IPC runtime is needed.
const mutateAsync = vi.fn();
vi.mock("@/ipc/queries/aiCompose", () => ({
  useGenerateComposeDraft: () => ({ mutateAsync, isPending: false }),
}));
vi.mock("@/components/ui/Toast", () => ({ showToast: vi.fn() }));
const navigateMock = vi.fn();
vi.mock("react-router-dom", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react-router-dom")>();
  return { ...actual, useNavigate: () => navigateMock };
});

const FWD =
  "\n\n---------- Forwarded message ----------\nFrom: Marcus <m@x.com>\nSubject: Quote\n\n$17,400 total.";

function wrapper({ children }: { children: React.ReactNode }) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return (
    <QueryClientProvider client={qc}>
      <MemoryRouter>{children}</MemoryRouter>
    </QueryClientProvider>
  );
}

beforeEach(() => {
  mutateAsync.mockReset();
  navigateMock.mockReset();
  useCompose.getState().reset();
  useCompose.setState({ accountId: "a1", to: "Sarah Chen <s.chen@x.com>", body: FWD });
});

describe("AiDraftButton", () => {
  it("renders nothing in reply mode (decision D3)", () => {
    const { container } = render(<AiDraftButton mode="reply" />, { wrapper });
    expect(container).toBeEmptyDOMElement();
  });

  it("disables Generate until an intent is chosen", () => {
    render(<AiDraftButton mode="forward" />, { wrapper });
    fireEvent.click(screen.getByTitle("AI Draft"));
    expect(screen.getByRole("button", { name: "Generate" })).toBeDisabled();
  });

  it("generates a forward note and inserts it above the quote", async () => {
    mutateAsync.mockResolvedValue({
      body: "Hi Sarah,\n\nPlease review the terms.",
      styleWasFallback: true,
    });
    render(<AiDraftButton mode="forward" />, { wrapper });

    fireEvent.click(screen.getByTitle("AI Draft"));
    fireEvent.click(screen.getByRole("button", { name: "Please review & advise" }));
    fireEvent.click(screen.getByRole("button", { name: "Generate" }));

    await waitFor(() => expect(mutateAsync).toHaveBeenCalledTimes(1));
    expect(mutateAsync).toHaveBeenCalledWith(
      expect.objectContaining({
        accountId: "a1",
        mode: "forward",
        intent: "review",
        tone: "Friendly",
        sourceExcerpt: expect.stringContaining("Forwarded message"),
      }),
    );

    await waitFor(() => expect(useCompose.getState().body.startsWith("Hi Sarah,")).toBe(true));
    expect(useCompose.getState().body).toContain("---------- Forwarded message ----------");
  });

  it("shows the sensitive disclosure when the forwarded text has an amount", () => {
    render(<AiDraftButton mode="forward" />, { wrapper });
    fireEvent.click(screen.getByTitle("AI Draft"));
    expect(screen.getByText(/sent to your AI provider/i)).toBeInTheDocument();
  });
});
