import { describe, it, expect } from "vitest";

import {
  requestCrossWindowSingleton,
  startSingletonResponder,
  type SingletonBus,
} from "./singletonBroadcast";

/** In-process synchronous bus standing in for Tauri's cross-window emit/listen. */
function makeFakeBus(): SingletonBus {
  const listeners = new Map<string, Set<(p: unknown) => void>>();
  return {
    emit: (event, payload) => {
      listeners.get(event)?.forEach((cb) => cb(payload));
    },
    listen: (event, cb) => {
      let set = listeners.get(event);
      if (!set) {
        set = new Set();
        listeners.set(event, set);
      }
      const fn = cb as (p: unknown) => void;
      set.add(fn);
      return Promise.resolve(() => set!.delete(fn));
    },
  };
}

describe("cross-window singleton broadcast (WB-17)", () => {
  it("resolves true and focuses the holder when another window claims the page", async () => {
    const bus = makeFakeBus();
    let focused = 0;
    await startSingletonResponder(
      () => true,
      bus,
      () => {
        focused += 1;
      },
    );
    expect(await requestCrossWindowSingleton("agent_im", bus, 50)).toBe(true);
    expect(focused).toBe(1);
  });

  it("resolves false when no window holds the page", async () => {
    const bus = makeFakeBus();
    expect(await requestCrossWindowSingleton("agent_im", bus, 5)).toBe(false);
  });

  it("a responder ignores pages it does not currently show", async () => {
    const bus = makeFakeBus();
    await startSingletonResponder(
      (page) => page === "dashboard",
      bus,
      () => {},
    );
    expect(await requestCrossWindowSingleton("agent_im", bus, 5)).toBe(false);
  });
});
