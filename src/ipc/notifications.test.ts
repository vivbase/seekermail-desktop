// T101 — notification level gating. The pure rule is tested in isolation; the OS
// send path is best-effort and a no-op off-Tauri, so it needs no runtime here.
import { describe, it, expect } from "vitest";

import { shouldNotifyQuery } from "./notifications";

describe("shouldNotifyQuery", () => {
  it("suppresses every notification when the level is off", () => {
    expect(shouldNotifyQuery("off", "high")).toBe(false);
    expect(shouldNotifyQuery("off", "normal")).toBe(false);
  });

  it("only allows high-priority notifications at the priority level", () => {
    expect(shouldNotifyQuery("priority", "high")).toBe(true);
    expect(shouldNotifyQuery("priority", "normal")).toBe(false);
  });

  it("allows all notifications at the all level", () => {
    expect(shouldNotifyQuery("all", "high")).toBe(true);
    expect(shouldNotifyQuery("all", "normal")).toBe(true);
  });
});
