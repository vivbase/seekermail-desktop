// T071 — `risk:alert` push event wiring: a new risk event must invalidate every
// ['riskEvents'] query so the T4 banner appears without a page refresh (02 §4).
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

describe("registerIpcEvents — risk:alert", () => {
  beforeEach(() => {
    listeners.clear();
    // Make isTauri() report true so the listener table actually registers.
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  });

  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  it("invalidates the riskEvents queries when a risk:alert arrives", async () => {
    const qc = new QueryClient();
    const invalidateSpy = vi.spyOn(qc, "invalidateQueries");

    const dispose = registerIpcEvents(qc);
    await Promise.resolve(); // let the listen() promises settle

    const handler = listeners.get("risk:alert");
    expect(handler).toBeDefined();

    handler?.({
      payload: {
        id: "risk-t4",
        mailId: "m-1",
        accountId: "demo-1",
        riskLevel: 4,
        riskType: "payment_anomaly",
        evidence: {},
        description: "Unusually long payment term.",
        status: "open",
        expiresAt: null,
        createdAt: 1,
      },
    });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ["riskEvents"] });
    dispose();
  });
});

describe("registerIpcEvents — query:new (T101)", () => {
  beforeEach(() => {
    listeners.clear();
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  });

  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  it("invalidates the pendingQueries and imMessages caches", async () => {
    const qc = new QueryClient();
    const invalidateSpy = vi.spyOn(qc, "invalidateQueries");

    const dispose = registerIpcEvents(qc);
    await Promise.resolve();

    const handler = listeners.get("query:new");
    expect(handler).toBeDefined();

    // Normal priority → no OS notification path, just cache invalidation.
    handler?.({ payload: { queryId: "q1", accountId: "demo-1", priority: "normal" } });

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ["pendingQueries"] });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ["imMessages"] });
    dispose();
  });
});
