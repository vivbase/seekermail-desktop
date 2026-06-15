// T090 — tokenized LCS diff unit tests.
import { describe, it, expect } from "vitest";

import { diffText, hasChanges, tokenize } from "./textDiff";

describe("tokenize", () => {
  it("splits words and whitespace into separate tokens", () => {
    expect(tokenize("Hi  there\nfriend")).toEqual(["Hi", "  ", "there", "\n", "friend"]);
  });

  it("returns an empty array for the empty string", () => {
    expect(tokenize("")).toEqual([]);
  });
});

describe("diffText", () => {
  it("returns one equal segment for identical inputs", () => {
    const segments = diffText("Hello world", "Hello world");
    expect(segments).toEqual([{ op: "equal", text: "Hello world" }]);
    expect(hasChanges(segments)).toBe(false);
  });

  it("returns [] for two empty strings", () => {
    expect(diffText("", "")).toEqual([]);
  });

  it("detects a pure insertion", () => {
    const segments = diffText("Hello", "Hello world");
    expect(segments).toEqual([
      { op: "equal", text: "Hello" },
      { op: "insert", text: " world" },
    ]);
    expect(hasChanges(segments)).toBe(true);
  });

  it("detects a pure deletion", () => {
    const segments = diffText("Hello big world", "Hello world");
    expect(
      segments
        .filter((s) => s.op === "delete")
        .map((s) => s.text)
        .join(""),
    ).toContain("big");
    // Reassembling equal+insert segments must yield the current text.
    expect(
      segments
        .filter((s) => s.op !== "delete")
        .map((s) => s.text)
        .join(""),
    ).toBe("Hello world");
  });

  it("detects a word replacement as delete + insert", () => {
    const segments = diffText("Send it on Friday", "Send it on Monday");
    expect(segments.some((s) => s.op === "delete" && s.text === "Friday")).toBe(true);
    expect(segments.some((s) => s.op === "insert" && s.text === "Monday")).toBe(true);
  });

  it("merges adjacent segments of the same op", () => {
    const segments = diffText("a b", "x y");
    for (let i = 1; i < segments.length; i++) {
      expect(segments[i]!.op).not.toBe(segments[i - 1]!.op);
    }
  });

  it("round-trips: deletes+equals rebuild the original, inserts+equals the current", () => {
    const original = "Hi Alice,\n\nThanks for the figures. Confirmed for the deck.\n\nBest";
    const current =
      "Hi Alice,\n\nThanks a lot for the figures. Confirmed and locked.\n\nBest,\nYou";
    const segments = diffText(original, current);
    expect(
      segments
        .filter((s) => s.op !== "insert")
        .map((s) => s.text)
        .join(""),
    ).toBe(original);
    expect(
      segments
        .filter((s) => s.op !== "delete")
        .map((s) => s.text)
        .join(""),
    ).toBe(current);
  });
});
