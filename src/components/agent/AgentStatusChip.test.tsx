// T102 — AgentStatusChip: primary ★, the processing spin animation, and the
// click-through to the TEAM channel.
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import type { Account } from "@shared/bindings";

import "@/i18n";

const navigateMock = vi.fn();
vi.mock("react-router-dom", async (orig) => {
  const actual = (await orig()) as Record<string, unknown>;
  return { ...actual, useNavigate: () => navigateMock };
});

import AgentStatusChip from "./AgentStatusChip";

const ACCOUNT: Account = {
  id: "demo-1",
  email: "you@example.com",
  displayName: "Work",
  provider: "imap",
  imapHost: null,
  imapPort: 993,
  smtpHost: null,
  smtpPort: 587,
  colorToken: "slate",
  badgeLabel: "W",
  roleType: "work",
  roleDescription: null,
  authLevel: 1,
  isPrimary: true,
  isActive: true,
  syncIntervalSecs: 300,
  lastSyncedAt: null,
  knowledgeDepthMonths: 12,
  createdAt: 0,
  updatedAt: 0,
};

beforeEach(() => navigateMock.mockClear());

describe("AgentStatusChip", () => {
  it("shows the ★ for a primary agent and navigates to /team on click", () => {
    render(<AgentStatusChip account={ACCOUNT} status="idle" />);
    expect(screen.getByLabelText("Primary agent")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button"));
    expect(navigateMock).toHaveBeenCalledWith("/team");
  });

  it("spins the presence dot when processing", () => {
    const { container } = render(<AgentStatusChip account={ACCOUNT} status="processing" />);
    expect(container.querySelector(".agent-status-dot--spinning")).not.toBeNull();
  });

  it("does not spin when idle", () => {
    const { container } = render(<AgentStatusChip account={ACCOUNT} status="idle" />);
    expect(container.querySelector(".agent-status-dot--spinning")).toBeNull();
  });
});
