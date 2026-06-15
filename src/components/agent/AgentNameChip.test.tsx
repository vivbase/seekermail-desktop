// T094 — AgentNameChip: primary ★, domain label, and the full-email tooltip.
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";

import "@/i18n";
import AgentNameChip from "./AgentNameChip";

describe("AgentNameChip", () => {
  it("shows the ★ and the full email tooltip for the primary agent", () => {
    render(<AgentNameChip displayName="Alex" email="alex@northwind.co" isPrimary />);
    expect(screen.getByLabelText("Primary agent")).toBeInTheDocument();
    expect(screen.getByText("Alex")).toBeInTheDocument();
    expect(screen.getByText("@northwind.co")).toBeInTheDocument();
    expect(screen.getByTitle("alex@northwind.co")).toBeInTheDocument();
  });

  it("omits the ★ for a non-primary agent", () => {
    render(<AgentNameChip displayName="Sam" email="sam@x.com" />);
    expect(screen.queryByLabelText("Primary agent")).not.toBeInTheDocument();
  });
});
