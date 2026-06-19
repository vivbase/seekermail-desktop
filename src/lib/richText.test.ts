// T044 — Plain-text ↔ HTML helpers for the rich-text compose editor (F_G4 §4.4).
import { describe, it, expect } from "vitest";

import { escapeHtml, plainTextToHtml, htmlToPlainText, isHtmlBlank } from "./richText";

describe("escapeHtml", () => {
  it("escapes the five HTML-significant characters", () => {
    expect(escapeHtml(`a & b < c > "d" 'e'`)).toBe(
      "a &amp; b &lt; c &gt; &quot;d&quot; &#39;e&#39;",
    );
  });

  it("leaves plain text untouched", () => {
    expect(escapeHtml("Hello Alex")).toBe("Hello Alex");
  });
});

describe("plainTextToHtml", () => {
  it("returns an empty string for empty input", () => {
    expect(plainTextToHtml("")).toBe("");
  });

  it("converts newlines to <br> and preserves blank lines", () => {
    expect(plainTextToHtml("a\nb")).toBe("a<br>b");
    expect(plainTextToHtml("a\n\nb")).toBe("a<br><br>b");
  });

  it("normalises CRLF line endings", () => {
    expect(plainTextToHtml("a\r\nb")).toBe("a<br>b");
  });

  it("escapes HTML so quoted markup cannot inject tags", () => {
    expect(plainTextToHtml("> <script>")).toBe("&gt; &lt;script&gt;");
  });
});

describe("htmlToPlainText", () => {
  it("turns <br> and block closers into newlines", () => {
    expect(htmlToPlainText("a<br>b")).toBe("a\nb");
    expect(htmlToPlainText("<div>a</div><div>b</div>")).toBe("a\nb");
  });

  it("strips tags and decodes common entities", () => {
    expect(htmlToPlainText(`<b>&lt;tag&gt;</b> &amp; &quot;x&quot;`)).toBe(`<tag> & "x"`);
  });

  it("round-trips with plainTextToHtml", () => {
    const source = "Hi Alex,\n\nThanks for the note — see < 3 items & reply.";
    expect(htmlToPlainText(plainTextToHtml(source))).toBe(source);
  });
});

describe("isHtmlBlank", () => {
  it("treats empty and structural-only markup as blank", () => {
    expect(isHtmlBlank("")).toBe(true);
    expect(isHtmlBlank("<br>")).toBe(true);
    expect(isHtmlBlank("<div><br></div>")).toBe(true);
    expect(isHtmlBlank("<p></p>")).toBe(true);
    expect(isHtmlBlank("   ")).toBe(true);
    expect(isHtmlBlank("&nbsp;")).toBe(true);
  });

  it("treats real content as non-blank", () => {
    expect(isHtmlBlank("<div>Hello</div>")).toBe(false);
    expect(isHtmlBlank("text")).toBe(false);
    expect(isHtmlBlank('<span style="color:#C0392B">x</span>')).toBe(false);
  });
});
