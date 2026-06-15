// T094 — AgentAvatar determinism + token-driven color. The same email must always
// render the identical SVG (identicon stability); different emails must differ.
import { describe, it, expect } from "vitest";
import { render } from "@testing-library/react";

import AgentAvatar from "./AgentAvatar";

describe("AgentAvatar", () => {
  it("is deterministic for the same email", () => {
    const a = render(<AgentAvatar email="a@b.com" colorToken="slate" />).container.innerHTML;
    const b = render(<AgentAvatar email="a@b.com" colorToken="slate" />).container.innerHTML;
    expect(a).toBe(b);
  });

  it("produces a different pattern for a different email", () => {
    const a = render(<AgentAvatar email="a@b.com" colorToken="slate" />).container.innerHTML;
    const c = render(<AgentAvatar email="c@d.org" colorToken="slate" />).container.innerHTML;
    expect(a).not.toBe(c);
  });

  it("paints the background from the color token (no bare hex)", () => {
    const { container } = render(<AgentAvatar email="a@b.com" colorToken="terra" />);
    expect(container.querySelector("rect")?.getAttribute("fill")).toBe("var(--terra)");
  });

  it("is marked decorative (aria-hidden) so the name chip owns the label", () => {
    const { container } = render(<AgentAvatar email="a@b.com" colorToken="sage" />);
    expect(container.querySelector("svg")?.getAttribute("aria-hidden")).toBe("true");
  });
});
