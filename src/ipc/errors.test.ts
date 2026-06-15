import { describe, it, expect } from "vitest";

import { ERROR_UX, uxForCode } from "./errors";
import enErrors from "@/i18n/resources/en/errors.json";

// 09 §8 "mapping completeness": every code in the table must have copy. The
// Record<ErrorCode, …> type already forces an entry per code at compile time; this
// test additionally asserts each entry's messageKey resolves to an `en` string.
describe("ERROR_UX table", () => {
  const codes = Object.keys(ERROR_UX) as (keyof typeof ERROR_UX)[];

  it("covers the full ErrorCode set", () => {
    // 21 wire codes (02 §2).
    expect(codes.length).toBe(21);
  });

  it("every entry has copy in en/errors.json", () => {
    const keys = new Set(Object.keys(enErrors));
    for (const code of codes) {
      const { messageKey, affordance, bucket } = ERROR_UX[code];
      expect(messageKey, `${code} messageKey`).toBeTruthy();
      expect(keys.has(messageKey), `missing en copy for ${messageKey}`).toBe(true);
      expect(affordance, `${code} affordance`).toBeTruthy();
      expect(bucket, `${code} bucket`).toBeTruthy();
    }
  });

  it("falls back to INTERNAL for an unknown code", () => {
    // @ts-expect-error — exercising the runtime default path.
    expect(uxForCode("NOPE")).toEqual(ERROR_UX.INTERNAL);
  });
});
