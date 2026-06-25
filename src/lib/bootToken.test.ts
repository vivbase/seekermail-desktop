import { describe, it, expect } from "vitest";

import { encodeBootToken, decodeBootToken } from "./bootToken";
import type { TabSpec } from "@/stores/workbench";

describe("bootToken (WB-19/20 ↔ WB-12 boundary)", () => {
  it("round-trips a workspace tab spec and stays URL-safe", () => {
    const spec: TabSpec = {
      route: { page: "thread", params: { mailId: "42" } },
      accountId: "acc-1",
    };
    const token = encodeBootToken(spec);
    expect(token).not.toMatch(/[+/=]/); // base64url: no +, /, or =
    expect(decodeBootToken(token)).toEqual(spec);
  });

  it("round-trips unicode params", () => {
    const spec: TabSpec = { route: { page: "search", params: { query: "survie 期限 ✓" } } };
    expect(decodeBootToken(encodeBootToken(spec))).toEqual(spec);
  });

  it("returns null for garbled or non-spec tokens", () => {
    expect(decodeBootToken("@@@not-base64@@@")).toBeNull();
    expect(decodeBootToken(encodeBootToken({} as unknown as TabSpec))).toBeNull(); // no route
  });
});
