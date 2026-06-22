// Regression (F_G3 §4.5): Reply / Reply all / Forward must navigate to
// /compose carrying router state { mode, mail, ownEmail } so the compose route
// can seed the From-account, recipients, subject, and quote block. A prior
// version pre-seeded the compose store then navigated WITHOUT state; the route's
// mount effect then called open() (empty) and wiped the seed, leaving the From
// account and To recipient blank. These tests pin the navigation payload.
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import React from "react";
import type { Account, MailDetail } from "@shared/bindings";

import "@/i18n";
import { MailToolbar } from "./MailToolbar";

// Capture navigation without leaving the test render tree.
const navigateMock = vi.fn();
vi.mock("react-router-dom", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react-router-dom")>();
  return { ...actual, useNavigate: () => navigateMock };
});

// The account that received MAIL — its address becomes the default From and is
// the ownEmail excluded from reply-all. Mocked so the query resolves
// synchronously (no async settle before the click).
const NOW = Math.floor(Date.now() / 1000);

const RECEIVING_ACCOUNT: Account = {
  id: "demo-1",
  email: "owner@example.com",
  displayName: "Owner",
  provider: "imap",
  imapHost: "imap.example.com",
  imapPort: 993,
  smtpHost: "smtp.example.com",
  smtpPort: 465,
  colorToken: "slate",
  badgeLabel: "W",
  roleType: "work",
  roleDescription: null,
  authLevel: 1,
  isPrimary: true,
  isActive: true,
  syncIntervalSecs: 300,
  lastSyncedAt: NOW,
  knowledgeDepthMonths: null,
  createdAt: NOW,
  updatedAt: NOW,
};

vi.mock("@/ipc/queries/accounts", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/ipc/queries/accounts")>();
  return { ...actual, useAccounts: () => ({ data: [RECEIVING_ACCOUNT] }) };
});

const MAIL: MailDetail = {
  id: "m-1",
  accountId: "demo-1",
  threadId: "t-1",
  subject: "Q4 budget review",
  fromName: "Alice Nguyen",
  fromEmail: "alice@northwind.co",
  to: [{ name: "Owner", email: "owner@example.com" }],
  cc: [{ name: "Bob", email: "bob@northwind.co" }],
  dateSent: NOW - 1800,
  bodyHtml: null,
  bodyText: "The revised figures are attached.",
  isRead: true,
  isStarred: false,
  isArchived: false,
  isDeleted: false,
  isSpam: false,
  hasAttachments: false,
  folder: "INBOX",
};

function wrapper({ children }: { children: React.ReactNode }) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return (
    <QueryClientProvider client={qc}>
      <MemoryRouter>{children}</MemoryRouter>
    </QueryClientProvider>
  );
}

beforeEach(() => {
  navigateMock.mockReset();
});

describe("MailToolbar — compose navigation carries router state", () => {
  it("Reply passes mode=reply, the mail, and the receiving account as ownEmail", () => {
    render(<MailToolbar mail={MAIL} />, { wrapper });
    fireEvent.click(screen.getByRole("button", { name: "Reply" }));
    expect(navigateMock).toHaveBeenCalledWith("/compose", {
      state: { mode: "reply", mail: MAIL, ownEmail: "owner@example.com" },
    });
  });

  it("Reply all passes mode=reply-all with the mail and ownEmail", () => {
    render(<MailToolbar mail={MAIL} />, { wrapper });
    fireEvent.click(screen.getByRole("button", { name: "Reply all" }));
    expect(navigateMock).toHaveBeenCalledWith("/compose", {
      state: { mode: "reply-all", mail: MAIL, ownEmail: "owner@example.com" },
    });
  });

  it("Forward passes mode=forward with the mail", () => {
    render(<MailToolbar mail={MAIL} />, { wrapper });
    fireEvent.click(screen.getByRole("button", { name: "Forward" }));
    expect(navigateMock).toHaveBeenCalledWith("/compose", {
      state: { mode: "forward", mail: MAIL, ownEmail: "owner@example.com" },
    });
  });
});

// A mail in the Trash view (analysis/44 §5) swaps Archive/Delete for Restore, so
// the user can recover it in-app instead of going back to the web.
describe("MailToolbar — Trash view shows Restore", () => {
  it("renders Restore (not Delete/Archive) for a trashed mail", () => {
    render(<MailToolbar mail={{ ...MAIL, isDeleted: true }} />, { wrapper });
    expect(screen.getByRole("button", { name: "Restore" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Delete" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Archive" })).toBeNull();
  });

  it("keeps Archive/Delete (no Restore) for a normal inbox mail", () => {
    render(<MailToolbar mail={MAIL} />, { wrapper });
    expect(screen.getByRole("button", { name: "Delete" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Restore" })).toBeNull();
  });
});

// A mail in the Spam view (analysis/44 §5) swaps Archive/Delete for "Not spam",
// which moves it back to the Inbox so the user can recover a false positive in-app.
describe("MailToolbar — Spam view shows Not spam", () => {
  it("renders Not spam (not Delete/Archive) for a spam'd mail", () => {
    render(<MailToolbar mail={{ ...MAIL, isSpam: true }} />, { wrapper });
    expect(screen.getByRole("button", { name: "Not spam" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Delete" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Archive" })).toBeNull();
  });
});
