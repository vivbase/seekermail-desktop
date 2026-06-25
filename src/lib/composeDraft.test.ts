import { describe, it, expect } from "vitest";

import { forwardedExcerpt, insertNoteAboveQuote, quoteOffset } from "./composeDraft";

const FWD =
  "\n\n---------- Forwarded message ----------\nFrom: Marcus <m@x.com>\nSubject: Quote\n\n$17,400 total.";

describe("quoteOffset", () => {
  it("finds the forwarded marker", () => {
    expect(quoteOffset(FWD)).toBeGreaterThan(0);
  });
  it("returns -1 with no quote", () => {
    expect(quoteOffset("plain body")).toBe(-1);
  });
});

describe("insertNoteAboveQuote", () => {
  it("places the note above the forwarded block", () => {
    const out = insertNoteAboveQuote(FWD, "Hi Sarah,\n\nPlease review.");
    expect(out.startsWith("Hi Sarah,")).toBe(true);
    expect(out).toContain("---------- Forwarded message ----------");
    expect(out.indexOf("Hi Sarah")).toBeLessThan(out.indexOf("Forwarded message"));
  });
  it("uses the note as the body when there is no quote", () => {
    expect(insertNoteAboveQuote("", "Hello.")).toBe("Hello.");
  });
  it("keeps user-typed text below the note when there is no quote", () => {
    expect(insertNoteAboveQuote("draft text", "Note.")).toBe("Note.\n\ndraft text");
  });
});

describe("forwardedExcerpt", () => {
  it("returns the quoted block, trimmed", () => {
    const ex = forwardedExcerpt(FWD);
    expect(ex.startsWith("----------")).toBe(true);
    expect(ex).toContain("Quote");
  });
  it("returns empty when there is no quote", () => {
    expect(forwardedExcerpt("just a body")).toBe("");
  });
  it("caps the length", () => {
    const long = `\n\n${"---------- Forwarded message ----------"}\n${"x".repeat(5000)}`;
    expect(forwardedExcerpt(long, 100).length).toBe(100);
  });
});
