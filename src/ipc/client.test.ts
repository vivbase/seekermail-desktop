import { describe, it, expect } from "vitest";

import { ipc, isTauri, normalizeIpcError } from "./client";

describe("ipc()", () => {
  it("runs in mock mode outside Tauri", () => {
    expect(isTauri()).toBe(false);
  });

  it("returns the ping fixture via the mock seam", async () => {
    const reply = await ipc("ping");
    expect(reply).toEqual({ message: "pong" });
  });
});

describe("normalizeIpcError()", () => {
  it("passes through a wire IpcError shape", () => {
    const e = normalizeIpcError({ code: "VALIDATION", message: "bad", detail: "field=x" });
    expect(e).toEqual({ code: "VALIDATION", message: "bad", detail: "field=x" });
  });

  it("folds a thrown Error into INTERNAL", () => {
    const e = normalizeIpcError(new Error("boom"));
    expect(e.code).toBe("INTERNAL");
    expect(e.message).toBe("boom");
    expect(e.detail).toBeNull();
  });

  it("folds an unknown throw into INTERNAL", () => {
    const e = normalizeIpcError("oops");
    expect(e.code).toBe("INTERNAL");
    expect(e.detail).toBeNull();
  });
});
