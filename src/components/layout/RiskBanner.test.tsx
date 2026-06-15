// T100 — the T4 risk banner: hidden with no open T4 events, shown (with no close
// button) for one, and a "+N more" count for several. The data hook is mocked so
// the count is deterministic.
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

import "@/i18n";
import type { RiskEvent } from "@/ipc/legal";

const h = vi.hoisted(() => ({ events: [] as RiskEvent[] }));
vi.mock("@/ipc/queries/risk", () => ({ useOpenRiskEvents: () => ({ data: h.events }) }));

import RiskBanner from "./RiskBanner";

function evt(id: string, level: number): RiskEvent {
  return {
    id,
    mailId: "m-1",
    accountId: "demo-1",
    riskLevel: level,
    riskType: "payment_anomaly",
    evidence: {},
    description: `Risk ${id}`,
    status: "open",
    expiresAt: null,
    createdAt: 0,
  };
}

function withRouter(ui: ReactNode) {
  return render(<MemoryRouter>{ui}</MemoryRouter>);
}

describe("RiskBanner", () => {
  it("renders nothing when there are no open T4 events", () => {
    h.events = [evt("r1", 3)]; // level 3 is not T4
    const { container } = withRouter(<RiskBanner />);
    expect(container.firstChild).toBeNull();
  });

  it("shows a non-dismissable alert for one T4 event", () => {
    h.events = [evt("r1", 4)];
    withRouter(<RiskBanner />);
    expect(screen.getByRole("alert")).toBeInTheDocument();
    // The hard rule: there is no close button.
    expect(screen.queryByRole("button", { name: /close/i })).toBeNull();
    expect(screen.getByRole("button", { name: "Review Now →" })).toBeInTheDocument();
  });

  it("shows a +N more count for several T4 events", () => {
    h.events = [evt("r1", 4), evt("r2", 4), evt("r3", 4)];
    withRouter(<RiskBanner />);
    expect(screen.getByText("+2 more")).toBeInTheDocument();
  });
});
