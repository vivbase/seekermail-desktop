import { describe, it, expect } from "vitest";

import { RevisionGate } from "./revisionGate";

describe("RevisionGate (WB-14)", () => {
  it("accepts strictly newer revisions and rejects stale/echoed ones", () => {
    const gate = new RevisionGate();
    expect(gate.current).toBe(-1);
    expect(gate.accept(1)).toBe(true);
    expect(gate.current).toBe(1);
    expect(gate.accept(1)).toBe(false); // echo of what we already applied
    expect(gate.accept(0)).toBe(false); // stale (out-of-order)
    expect(gate.accept(5)).toBe(true);
    expect(gate.current).toBe(5);
  });

  it("resets", () => {
    const gate = new RevisionGate();
    gate.accept(3);
    gate.reset();
    expect(gate.current).toBe(-1);
    expect(gate.accept(1)).toBe(true);
  });
});
