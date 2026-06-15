// T073 tests — AgentCard role/auth editing, the Full-Auto confirmation
// intercept, the 500-char role_description soft limit, and the AI-settings
// hooks. Off-Tauri, `ipc()` resolves from the stateful mock layer in client.ts.
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import React from "react";
import type { Account } from "@shared/bindings";

import "@/i18n";
import * as client from "@/ipc/client";
import { useAccountAiSettings, accountKeys } from "@/ipc/queries/accounts";
import AgentCard from "./AgentCard";

const LEGAL_ACCOUNT: Account = {
  id: "acct-legal-1",
  email: "legal@northwind.co",
  displayName: "Legal",
  provider: "imap",
  imapHost: "imap.northwind.co",
  imapPort: 993,
  smtpHost: "smtp.northwind.co",
  smtpPort: 587,
  colorToken: "terra",
  badgeLabel: "L",
  roleType: "legal",
  roleDescription: "Reviews inbound contracts and flags liability clauses.",
  authLevel: 1,
  isPrimary: true,
  isActive: true,
  syncIntervalSecs: 300,
  lastSyncedAt: null,
  knowledgeDepthMonths: 12,
  createdAt: 0,
  updatedAt: 0,
};

function newQueryClient() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
}

function renderCard(onSaved = vi.fn()) {
  const qc = newQueryClient();
  render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <AgentCard account={LEGAL_ACCOUNT} onSaved={onSaved} />
      </MemoryRouter>
    </QueryClientProvider>,
  );
  return { qc, onSaved };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("AgentCard — role_type accent", () => {
  it("uses the terra accent for a legal agent and follows a role_type change", () => {
    renderCard();
    const card = screen.getByRole("article", { name: "Legal" });
    expect(card.style.getPropertyValue("--agent-accent")).toBe("var(--terra)");

    fireEvent.change(screen.getByLabelText("Role Type"), { target: { value: "work" } });
    expect(card.style.getPropertyValue("--agent-accent")).toBe("var(--slate)");
  });
});

describe("AgentCard — role_description soft limit", () => {
  it("disables Save and warns when the description exceeds 500 characters", () => {
    renderCard();
    fireEvent.change(screen.getByLabelText("Role Description"), {
      target: { value: "a".repeat(501) },
    });
    expect(screen.getByRole("button", { name: "Save Changes" })).toBeDisabled();
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });
});

describe("AgentCard — Full Auto confirmation", () => {
  it("intercepts the switch to Full Auto; Cancel keeps the previous level", () => {
    renderCard();
    const fullAuto = screen.getByRole("radio", { name: "Full Auto" });
    fireEvent.click(fullAuto);
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Full Auto" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
  });

  it("applies level 3 after Confirm", () => {
    renderCard();
    fireEvent.click(screen.getByRole("radio", { name: "Full Auto" }));
    fireEvent.click(screen.getByRole("button", { name: "Enable Full Auto" }));
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Full Auto" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
  });
});

describe("AgentCard — save", () => {
  it("calls update_account then update_account_ai_settings and reports success", async () => {
    const ipcSpy = vi.spyOn(client, "ipc");
    const { onSaved } = renderCard();

    fireEvent.change(screen.getByLabelText("Role Type"), { target: { value: "work" } });
    fireEvent.click(screen.getByRole("button", { name: "Save Changes" }));

    await waitFor(() => expect(onSaved).toHaveBeenCalledWith("Agent settings saved."));

    const commands = ipcSpy.mock.calls.map((c) => c[0]);
    const accountIdx = commands.indexOf("update_account");
    const aiIdx = commands.indexOf("update_account_ai_settings");
    expect(accountIdx).toBeGreaterThanOrEqual(0);
    expect(aiIdx).toBeGreaterThan(accountIdx);

    const aiArgs = ipcSpy.mock.calls[aiIdx]?.[1] as
      | { account_id: string; params: { authLevel: number | null } }
      | undefined;
    expect(aiArgs?.account_id).toBe(LEGAL_ACCOUNT.id);
    expect(aiArgs?.params.authLevel).toBe(LEGAL_ACCOUNT.authLevel);
  });
});

describe("AgentCard — set primary (T091)", () => {
  it("shows the ★ marker and the Primary badge for the primary account", () => {
    renderCard();
    expect(screen.getByLabelText("Primary")).toBeInTheDocument();
    // No promote button when the account is already primary.
    expect(screen.queryByRole("button", { name: "Set as Primary" })).not.toBeInTheDocument();
  });

  it("promotes a non-primary account after confirmation", async () => {
    const ipcSpy = vi.spyOn(client, "ipc");
    const qc = newQueryClient();
    render(
      <QueryClientProvider client={qc}>
        <MemoryRouter>
          <AgentCard account={{ ...LEGAL_ACCOUNT, isPrimary: false }} onSaved={vi.fn()} />
        </MemoryRouter>
      </QueryClientProvider>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Set as Primary" }));
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Make Primary" }));
    await waitFor(() =>
      expect(ipcSpy.mock.calls.some((c) => c[0] === "set_primary_account")).toBe(true),
    );
    const call = ipcSpy.mock.calls.find((c) => c[0] === "set_primary_account");
    expect((call?.[1] as { account_id: string }).account_id).toBe(LEGAL_ACCOUNT.id);
  });

  it("keeps the previous primary when the dialog is cancelled", () => {
    const qc = newQueryClient();
    render(
      <QueryClientProvider client={qc}>
        <MemoryRouter>
          <AgentCard account={{ ...LEGAL_ACCOUNT, isPrimary: false }} onSaved={vi.fn()} />
        </MemoryRouter>
      </QueryClientProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Set as Primary" }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });
});

describe("useAccountAiSettings", () => {
  it("fetches the per-account row under the accountAiSettings query key", async () => {
    const qc = newQueryClient();
    const wrapper = ({ children }: { children: React.ReactNode }) =>
      React.createElement(QueryClientProvider, { client: qc }, children);

    const { result } = renderHook(() => useAccountAiSettings("acct-legal-1"), { wrapper });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(result.current.data?.accountId).toBe("acct-legal-1");
    expect(qc.getQueryData(accountKeys.aiSettings("acct-legal-1"))).toBeDefined();
  });
});
