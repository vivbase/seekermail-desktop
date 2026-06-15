// T102 — AgentBadgeRow: one chip per account, primary first, and nothing when
// there are no accounts. The data hooks are mocked so ordering is deterministic.
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import type { Account } from "@shared/bindings";

import "@/i18n";

const h = vi.hoisted(() => ({
  accounts: [] as Account[],
  statuses: [] as { accountId: string; status: string }[],
}));

vi.mock("@/ipc/queries/accounts", () => ({ useAccounts: () => ({ data: h.accounts }) }));
vi.mock("@/ipc/queries/agents", () => ({ useAgentStatuses: () => ({ data: h.statuses }) }));

import AgentBadgeRow from "./AgentBadgeRow";

function acct(over: Partial<Account>): Account {
  return {
    id: "x",
    email: "x@y.com",
    displayName: "X",
    provider: "imap",
    imapHost: null,
    imapPort: 993,
    smtpHost: null,
    smtpPort: 587,
    colorToken: "slate",
    badgeLabel: "X",
    roleType: "work",
    roleDescription: null,
    authLevel: 1,
    isPrimary: false,
    isActive: true,
    syncIntervalSecs: 300,
    lastSyncedAt: null,
    knowledgeDepthMonths: null,
    createdAt: 0,
    updatedAt: 0,
    ...over,
  };
}

describe("AgentBadgeRow", () => {
  it("renders nothing when there are no accounts", () => {
    h.accounts = [];
    h.statuses = [];
    const { container } = render(
      <MemoryRouter>
        <AgentBadgeRow />
      </MemoryRouter>,
    );
    expect(container.firstChild).toBeNull();
  });

  it("renders one chip per account with the primary agent first", () => {
    h.accounts = [
      acct({ id: "a2", email: "work@x.com", displayName: "Work", isPrimary: false, createdAt: 1 }),
      acct({ id: "a1", email: "legal@x.com", displayName: "Legal", isPrimary: true, createdAt: 2 }),
    ];
    h.statuses = [
      { accountId: "a1", status: "idle" },
      { accountId: "a2", status: "offline" },
    ];
    render(
      <MemoryRouter>
        <AgentBadgeRow />
      </MemoryRouter>,
    );
    const buttons = screen.getAllByRole("button");
    expect(buttons).toHaveLength(2);
    // Primary (Legal) is sorted ahead of the non-primary Work account.
    expect(buttons[0]).toHaveAccessibleName(/Legal/);
  });
});
