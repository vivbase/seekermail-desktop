// T090 — DraftDiffView render states: insert/delete markup via React elements
// and the "No edits made" hint for an untouched draft.
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";

import "@/i18n";
import { DraftDiffView } from "./DraftDiffView";

describe("DraftDiffView", () => {
  it("renders inserted text inside an <ins> element", () => {
    const { container } = render(<DraftDiffView original="Hello" current="Hello world" />);
    const ins = container.querySelector("ins");
    expect(ins).not.toBeNull();
    expect(ins?.textContent).toBe(" world");
    expect(container.querySelector("del")).toBeNull();
  });

  it("renders deleted text inside a struck-through <del> element", () => {
    const { container } = render(
      <DraftDiffView original="Send on Friday please" current="Send on Monday please" />,
    );
    const del = container.querySelector("del");
    const ins = container.querySelector("ins");
    expect(del?.textContent).toBe("Friday");
    expect(ins?.textContent).toBe("Monday");
    expect(del?.className).toContain("line-through");
  });

  it("shows the no-edits hint (and no ins/del) when original equals current", () => {
    const { container } = render(<DraftDiffView original="Same text" current="Same text" />);
    expect(screen.getByText("No edits made — original AI draft")).toBeInTheDocument();
    expect(container.querySelector("ins")).toBeNull();
    expect(container.querySelector("del")).toBeNull();
  });

  it("never uses dangerouslySetInnerHTML — markup-looking text stays literal", () => {
    const { container } = render(
      <DraftDiffView original="plain" current="plain <script>alert(1)</script>" />,
    );
    expect(container.querySelector("script")).toBeNull();
    expect(container.textContent).toContain("<script>alert(1)</script>");
  });
});
