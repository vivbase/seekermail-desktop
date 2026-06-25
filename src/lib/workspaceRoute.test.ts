import { describe, it, expect } from "vitest";

import { routeToPath, pathToRoute } from "./workspaceRoute";
import type { WorkspaceRoute } from "@/stores/workbench";

describe("workspaceRoute mapping (WB-09)", () => {
  it("maps routes to paths", () => {
    expect(routeToPath({ page: "dashboard" })).toBe("/");
    expect(routeToPath({ page: "inbox" })).toBe("/all-mail");
    expect(routeToPath({ page: "agent_im" })).toBe("/team");
    expect(routeToPath({ page: "search" })).toBe("/search");
    expect(routeToPath({ page: "thread", params: { mailId: "42" } })).toBe("/mail/42");
    expect(routeToPath({ page: "inbox", params: { accountId: "ac1" } })).toBe("/accounts/ac1/mail");
  });

  it("maps paths back to routes", () => {
    expect(pathToRoute("/")).toEqual({ page: "dashboard" });
    expect(pathToRoute("/all-mail")).toEqual({ page: "inbox" });
    expect(pathToRoute("/team")).toEqual({ page: "agent_im" });
    expect(pathToRoute("/mail/42")).toEqual({ page: "thread", params: { mailId: "42" } });
    expect(pathToRoute("/accounts/ac1/mail")).toEqual({
      page: "inbox",
      params: { accountId: "ac1" },
    });
    expect(pathToRoute("/settings/privacy")).toEqual({ page: "settings" });
  });

  it("returns null for non-workspace paths", () => {
    expect(pathToRoute("/onboarding")).toBeNull();
    expect(pathToRoute("/totally-unknown")).toBeNull();
  });

  it("round-trips the primary pages", () => {
    const routes: WorkspaceRoute[] = [
      { page: "dashboard" },
      { page: "inbox" },
      { page: "search" },
      { page: "agent_im" },
      { page: "agents" },
      { page: "repository" },
      { page: "pending" },
      { page: "compose" },
      { page: "thread", params: { mailId: "7" } },
    ];
    for (const r of routes) {
      expect(pathToRoute(routeToPath(r))).toEqual(r);
    }
  });
});
