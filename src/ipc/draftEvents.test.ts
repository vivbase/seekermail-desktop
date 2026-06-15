// T081 — Module E draft push events: draft:ready / draft:updated /
// draft:discarded must invalidate the Pending queries so cards appear and
// disappear without a refresh (02 §4, F_E6 §4.3).
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { QueryClient } from "@tanstack/react-query";

// Capture registered Tauri listeners; the factory is hoisted, so the registry
// must come from vi.hoisted().
const { listeners } = vi.hoisted(() => ({
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (event: { payload: unknown }) => void) => {
    listeners.set(name, cb);
    return Promise.resolve(() => listeners.delete(name));
  }),
}));

import { registerIpcEvents } from "./events";

describe("registerIpcEvents — Module E draft events", () => {
  beforeEach(() => {
    listeners.clear();
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  });

  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  it("invalidates the pending-draft queries on draft:ready", async () => {
    const qc = new QueryClient();
    const invalidateSpy = vi.spyOn(qc, "invalidateQueries");

    const dispose = registerIpcEvents(qc);
    await Promise.resolve();

    const handler = listeners.get("draft:ready");
    expect(handler).toBeDefined();
    handler?.({
      payload: {
        draftId: "ai-draft-1",
        mailId: "m-1",
        triggerMode: "E2_semi",
        accountId: "demo-1",
      },
    });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ["pending_drafts"] });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ["pending_counts"] });
    dispose();
  });

  it("invalidates the draft detail + list on draft:discarded", async () => {
    const qc = new QueryClient();
    const invalidateSpy = vi.spyOn(qc, "invalidateQueries");

    const dispose = registerIpcEvents(qc);
    await Promise.resolve();

    const handler = listeners.get("draft:discarded");
    expect(handler).toBeDefined();
    handler?.({ payload: { draftId: "ai-draft-1", reason: "expired" } });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ["ai_draft", "ai-draft-1"] });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ["pending_drafts"] });
    dispose();
  });

  it("invalidates on draft:updated", async () => {
    const qc = new QueryClient();
    const invalidateSpy = vi.spyOn(qc, "invalidateQueries");

    const dispose = registerIpcEvents(qc);
    await Promise.resolve();

    listeners.get("draft:updated")?.({ payload: { draftId: "ai-draft-1" } });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ["pending_drafts"] });
    dispose();
  });
});
