import { describe, it, expect, beforeEach } from "vitest";

import { useUi } from "./ui";

describe("ui store", () => {
  beforeEach(() => {
    useUi.setState({ activeRoute: "/", agentRailOpen: true, density: "comfortable" });
  });

  it("toggles the agent rail", () => {
    expect(useUi.getState().agentRailOpen).toBe(true);
    useUi.getState().toggleAgentRail();
    expect(useUi.getState().agentRailOpen).toBe(false);
  });

  it("mirrors the active route", () => {
    useUi.getState().setActiveRoute("/pending");
    expect(useUi.getState().activeRoute).toBe("/pending");
  });

  it("sets density", () => {
    useUi.getState().setDensity("compact");
    expect(useUi.getState().density).toBe("compact");
  });
});
