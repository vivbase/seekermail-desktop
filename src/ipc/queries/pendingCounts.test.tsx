// T083 — usePendingCounts derives badge counts from the cached pending-drafts
// query (no extra IPC command).
import { describe, it, expect, vi, afterEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { AiDraft } from "@shared/bindings";

import * as client from "@/ipc/client";
import { usePendingCounts, useAiDraftTriggerMailIds } from "./drafts";

const NOW = Math.floor(Date.now() / 1000);

function makeDraft(id: string, triggerMailId: string): AiDraft {
  return {
    id,
    triggerMailId,
    accountId: "demo-1",
    toAddr: { name: "Alice Nguyen", email: "alice@northwind.co" },
    ccAddrs: [],
    subject: `Re: thread ${id}`,
    bodyOriginal: "Hi Alice,\n\nConfirmed.\n\nBest",
    bodyCurrent: "Hi Alice,\n\nConfirmed.\n\nBest",
    isEdited: false,
    styleMatchScore: null,
    triggerMode: "E2_semi",
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
}

function wrapper({ children }: { children: React.ReactNode }) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("usePendingCounts (T083)", () => {
  it("reports draftCount = 3 when three drafts are pending", async () => {
    vi.spyOn(client, "ipc").mockResolvedValue([
      makeDraft("d-1", "m-1"),
      makeDraft("d-2", "m-2"),
      makeDraft("d-3", "m-3"),
    ]);
    const { result } = renderHook(() => usePendingCounts(), { wrapper });
    await waitFor(() => expect(result.current.draftCount).toBe(3));
    // Decision queries land with T095/T096 — until then always zero.
    expect(result.current.decisionCount).toBe(0);
  });

  it("reports zero counts for an empty pending list", async () => {
    vi.spyOn(client, "ipc").mockResolvedValue([]);
    const { result } = renderHook(() => usePendingCounts(), { wrapper });
    await waitFor(() => expect(result.current.draftCount).toBe(0));
    expect(result.current.decisionCount).toBe(0);
  });
});

describe("useAiDraftTriggerMailIds (T083 L0 badge)", () => {
  it("exposes an O(1) membership set keyed by triggerMailId", async () => {
    vi.spyOn(client, "ipc").mockResolvedValue([makeDraft("d-1", "m-1")]);
    const { result } = renderHook(() => useAiDraftTriggerMailIds(), { wrapper });
    await waitFor(() => expect(result.current.has("m-1")).toBe(true));
    expect(result.current.has("m-2")).toBe(false);
  });
});
