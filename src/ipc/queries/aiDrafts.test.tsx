// T078/T081 — Module E AI-draft hooks against a mocked `ipc`. Covers the E1
// success path (navigate to /compose with the AI seed), the failure fallbacks
// (blank reply / AI settings), the pending-draft list, and approve/regenerate.
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import React from "react";
import type { AiDraft, MailDetail } from "@shared/bindings";

import "@/i18n";
import * as client from "@/ipc/client";
import { useToastStore } from "@/components/ui/Toast";
import { useApproveDraft, usePendingDrafts, useRegenerateDraft, useRequestAiReply } from "./drafts";

const navigateMock = vi.fn();
vi.mock("react-router-dom", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react-router-dom")>();
  return { ...actual, useNavigate: () => navigateMock };
});

const NOW = Math.floor(Date.now() / 1000);

const MAIL: MailDetail = {
  id: "m-1",
  accountId: "demo-1",
  threadId: "t-1",
  subject: "Q4 budget review — final numbers",
  fromName: "Alice Nguyen",
  fromEmail: "alice@northwind.co",
  to: [{ name: "You", email: "you@example.com" }],
  cc: [],
  dateSent: NOW - 1800,
  bodyHtml: "<p>The revised figures are attached.</p>",
  bodyText: "The revised figures are attached.",
  isRead: true,
  isStarred: false,
  isArchived: false,
  isDeleted: false,
  isSpam: false,
  hasAttachments: false,
  folder: "INBOX",
};

const DRAFT: AiDraft = {
  id: "ai-draft-9",
  triggerMailId: "m-1",
  accountId: "demo-1",
  toAddr: { name: "Alice Nguyen", email: "alice@northwind.co" },
  ccAddrs: [],
  subject: "Re: Q4 budget review — final numbers",
  bodyOriginal: "Hi Alice,\n\n**Confirmed** for the board deck.\n\nBest",
  bodyCurrent: "Hi Alice,\n\n**Confirmed** for the board deck.\n\nBest",
  isEdited: false,
  styleMatchScore: 0.9,
  triggerMode: "E1_manual",
  aiModel: "mock-model",
  knowledgeRefs: [],
  status: "pending",
  sendAfter: null,
  expiresAt: NOW + 72 * 3600,
  sentAt: null,
  discardedAt: null,
  discardReason: null,
  createdAt: NOW,
  updatedAt: NOW,
};

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
  navigateMock.mockReset();
  useToastStore.setState({ toasts: [] });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("useRequestAiReply (T078)", () => {
  it("navigates to /compose with the AI seed on success", async () => {
    const ipcSpy = vi.spyOn(client, "ipc").mockResolvedValue(DRAFT);
    const { result } = renderHook(() => useRequestAiReply(), { wrapper });

    result.current.mutate({ mail: MAIL });

    await waitFor(() => expect(navigateMock).toHaveBeenCalled());
    expect(ipcSpy).toHaveBeenCalledWith("request_ai_reply", {
      params: { mailId: "m-1", instruction: null },
    });
    const [path, options] = navigateMock.mock.calls[0] as [
      string,
      { state: { mode: string; aiSeed: { body: string; aiDraftId: string } } },
    ];
    expect(path).toBe("/compose");
    expect(options.state.mode).toBe("reply");
    expect(options.state.aiSeed.aiDraftId).toBe(DRAFT.id);
    // Markdown bold markers are stripped for the plain-text editor.
    expect(options.state.aiSeed.body).toContain("Confirmed for the board deck");
    expect(options.state.aiSeed.body.length).toBeGreaterThan(0);
  });

  it("reply-all scope widens the seed To to sender + Cc and sets mode=reply-all", async () => {
    vi.spyOn(client, "ipc").mockResolvedValue(DRAFT);
    const { result } = renderHook(() => useRequestAiReply(), { wrapper });

    const MAIL_MULTI: MailDetail = {
      ...MAIL,
      cc: [{ name: "Bob", email: "bob@northwind.co" }],
    };
    result.current.mutate({ mail: MAIL_MULTI, scope: "reply-all", ownEmail: "you@example.com" });

    await waitFor(() => expect(navigateMock).toHaveBeenCalled());
    const [path, options] = navigateMock.mock.calls[0] as [
      string,
      { state: { mode: string; aiSeed: { to: string } } },
    ];
    expect(path).toBe("/compose");
    expect(options.state.mode).toBe("reply-all");
    // Reply-all keeps the sender and adds the Cc recipient; ownEmail is excluded.
    expect(options.state.aiSeed.to).toContain("alice@northwind.co");
    expect(options.state.aiSeed.to).toContain("bob@northwind.co");
    expect(options.state.aiSeed.to).not.toContain("you@example.com");
  });

  it("falls back to a blank reply compose + toast on generation failure", async () => {
    vi.spyOn(client, "ipc").mockRejectedValue({
      code: "AI_PROVIDER_UNREACHABLE",
      message: "Provider timed out.",
      detail: null,
    });
    const { result } = renderHook(() => useRequestAiReply(), { wrapper });

    result.current.mutate({ mail: MAIL });

    await waitFor(() => expect(navigateMock).toHaveBeenCalled());
    const [path, options] = navigateMock.mock.calls[0] as [
      string,
      { state: { mode: string; mail: MailDetail; aiSeed?: unknown } },
    ];
    expect(path).toBe("/compose");
    expect(options.state.mail.id).toBe("m-1");
    expect(options.state.aiSeed).toBeUndefined();
    expect(useToastStore.getState().toasts.some((t) => t.message.includes("failed"))).toBe(true);
  });

  it("routes to AI settings when no provider is configured", async () => {
    vi.spyOn(client, "ipc").mockRejectedValue({
      code: "AI_PROVIDER_UNREACHABLE",
      message: "No AI provider configured.",
      detail: "ai_provider not_configured for account demo-1",
    });
    const { result } = renderHook(() => useRequestAiReply(), { wrapper });

    result.current.mutate({ mail: MAIL });

    await waitFor(() => expect(navigateMock).toHaveBeenCalledWith("/settings/ai"));
    // The compose window must NOT open in this path.
    expect(navigateMock).not.toHaveBeenCalledWith("/compose", expect.anything());
    expect(useToastStore.getState().toasts.length).toBeGreaterThan(0);
  });
});

describe("usePendingDrafts (T081)", () => {
  it("fetches the pending list through list_pending_drafts", async () => {
    const ipcSpy = vi.spyOn(client, "ipc").mockResolvedValue([DRAFT]);
    const { result } = renderHook(() => usePendingDrafts(), { wrapper });

    await waitFor(() => expect(result.current.data).toHaveLength(1));
    expect(ipcSpy).toHaveBeenCalledWith("list_pending_drafts", {
      params: { accountId: null, limit: null },
    });
    expect(result.current.data?.[0]?.id).toBe(DRAFT.id);
  });
});

describe("useApproveDraft (T081)", () => {
  it("returns the pendingId used by the undo window", async () => {
    const ipcSpy = vi.spyOn(client, "ipc").mockResolvedValue({
      sentAt: NOW,
      messageId: "<x@seekermail.local>",
      pendingId: "pending-77",
    });
    const { result } = renderHook(() => useApproveDraft(), { wrapper });

    const res = await result.current.mutateAsync("ai-draft-9");
    expect(ipcSpy).toHaveBeenCalledWith("approve_draft", { id: "ai-draft-9" });
    expect(res.pendingId).toBe("pending-77");
  });

  it("surfaces SMTP failures to the caller", async () => {
    vi.spyOn(client, "ipc").mockRejectedValue({
      code: "SMTP_SEND_FAILED",
      message: "The message couldn't be sent.",
      detail: null,
    });
    const { result } = renderHook(() => useApproveDraft(), { wrapper });

    await expect(result.current.mutateAsync("ai-draft-9")).rejects.toMatchObject({
      code: "SMTP_SEND_FAILED",
    });
  });
});

describe("useRegenerateDraft (T078/T081)", () => {
  it("returns the regenerated draft", async () => {
    const regenerated = { ...DRAFT, bodyCurrent: "Hi Alice,\n\nShorter answer.\n\nBest" };
    const ipcSpy = vi.spyOn(client, "ipc").mockResolvedValue(regenerated);
    const { result } = renderHook(() => useRegenerateDraft(), { wrapper });

    const res = await result.current.mutateAsync({ id: DRAFT.id });
    expect(ipcSpy).toHaveBeenCalledWith("regenerate_draft", {
      params: { id: DRAFT.id, instruction: null },
    });
    expect(res.bodyCurrent).toContain("Shorter answer");
  });
});
