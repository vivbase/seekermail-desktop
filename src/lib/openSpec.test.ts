import { describe, it, expect } from "vitest";

import { openSpecAttr, parseOpenSpec, OPEN_SPEC_ATTR } from "./openSpec";
import type { TabSpec } from "@/stores/workbench";

describe("openSpec (WB-19)", () => {
  it("round-trips a tab spec through the data attribute", () => {
    const spec: TabSpec = { route: { page: "thread", params: { mailId: "42" } }, accountId: "x" };
    const attr = openSpecAttr(spec);
    expect(Object.keys(attr)).toEqual([OPEN_SPEC_ATTR]);
    expect(parseOpenSpec(attr[OPEN_SPEC_ATTR])).toEqual(spec);
  });

  it("returns null for missing or garbled values", () => {
    expect(parseOpenSpec(null)).toBeNull();
    expect(parseOpenSpec("")).toBeNull();
    expect(parseOpenSpec("{not json")).toBeNull();
    expect(parseOpenSpec(JSON.stringify({ nope: 1 }))).toBeNull();
  });
});
