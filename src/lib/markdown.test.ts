// T078 — Markdown handling for AI draft bodies (F_E1 §4.3).
import { describe, it, expect } from "vitest";

import { markdownToHtml, markdownToPlainText, stripCodeFences } from "./markdown";

describe("stripCodeFences", () => {
  it("removes fence lines but keeps the content between them", () => {
    expect(stripCodeFences("```\nHello there\n```")).toBe("Hello there");
  });

  it("leaves fence-free text untouched", () => {
    expect(stripCodeFences("Hi Alice,\n\nBest")).toBe("Hi Alice,\n\nBest");
  });
});

describe("markdownToHtml", () => {
  it("converts bold, paragraphs, and line breaks", () => {
    const html = markdownToHtml("Hi **Alice**,\nsecond line\n\nBest");
    expect(html).toBe("<p>Hi <strong>Alice</strong>,<br>second line</p><p>Best</p>");
  });

  it("escapes HTML in the model output", () => {
    expect(markdownToHtml("<script>alert(1)</script>")).toBe(
      "<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>",
    );
  });

  it("returns an empty string for empty input", () => {
    expect(markdownToHtml("")).toBe("");
  });
});

describe("markdownToPlainText", () => {
  it("drops bold markers and code fences", () => {
    expect(markdownToPlainText("```\nHi **Alice**\n```")).toBe("Hi Alice");
  });
});
