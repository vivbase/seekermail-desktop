// Tests for the inline-style scrubber (defence-in-depth, mirrors the Rust
// `sanitize::scrub_style`). Verifies safe presentational CSS survives while
// every remote-load / breakout vector is dropped.
import { describe, it, expect } from "vitest";

import { scrubInlineStyle } from "./cssScrub";

describe("scrubInlineStyle — keeps safe presentational CSS", () => {
  it("preserves colour, spacing, font and border declarations", () => {
    const out = scrubInlineStyle(
      "color: #333; padding: 10px 14px; font-size: 13px; border: 1px solid #ccc",
    );
    expect(out).toContain("color: #333");
    expect(out).toContain("padding: 10px 14px");
    expect(out).toContain("font-size: 13px");
    expect(out).toContain("border: 1px solid #ccc");
  });

  it("lowercases the property but leaves the value untouched", () => {
    expect(scrubInlineStyle("COLOR: Red")).toBe("color: Red");
  });

  it("normalises declaration spacing and trims empties", () => {
    expect(scrubInlineStyle("color:red;;  padding: 4px ;")).toBe("color: red; padding: 4px");
  });
});

describe("scrubInlineStyle — drops dangerous declarations", () => {
  it("strips url() so blocked remote requests cannot return via CSS", () => {
    expect(scrubInlineStyle("background-color: url(http://tracker.example/p.gif)")).toBe("");
    expect(scrubInlineStyle("color: red; background-color: url(x)")).toBe("color: red");
  });

  it("drops positioning so a message can't overlay the app chrome", () => {
    expect(scrubInlineStyle("position: fixed; top: 0; color: red")).toBe("color: red");
  });

  it("rejects expression(), javascript:, @import and CSS escapes", () => {
    expect(scrubInlineStyle("width: expression(alert(1))")).toBe("");
    expect(scrubInlineStyle("background-color: javascript:alert(1)")).toBe("");
    expect(scrubInlineStyle("color: red; @import: 'evil.css'")).toBe("color: red");
    expect(scrubInlineStyle("color: \\75 rl(x)")).toBe("");
  });

  it("ignores unknown / non-allowlisted properties", () => {
    expect(scrubInlineStyle("behavior: url(x.htc); -moz-binding: url(x); color: red")).toBe(
      "color: red",
    );
  });

  it("returns empty for nullish or borderline input", () => {
    expect(scrubInlineStyle(null)).toBe("");
    expect(scrubInlineStyle(undefined)).toBe("");
    expect(scrubInlineStyle("")).toBe("");
    expect(scrubInlineStyle("not-a-declaration")).toBe("");
  });
});
