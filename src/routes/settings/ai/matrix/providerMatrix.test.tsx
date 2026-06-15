// T066 tests — provider-matrix grid rendering (accounts × capabilities), the
// cell popover editor's client-side validation (backup cap, primary≠backup),
// warning rendering after a save, the batch toolbar wiring, the reset flow,
// and the simplified single-column mode. Off-Tauri, `ipc()` resolves from the
// stateful mock layer in client.ts unless a test overrides commands in-place.
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { Account } from "@shared/bindings";

import "@/i18n";
import * as client from "@/ipc/client";
import type { BatchMatrixUpdate, MatrixWarning } from "@/ipc/aiMatrix";
import type { ConfiguredProviderInfo } from "@/ipc/aiSettings";
import ProviderMatrixPage from "./index";

function account(id: string, email: string): Account {
  return {
    id,
    email,
    displayName: "Work",
    provider: "imap",
    imapHost: "imap.northwind.co",
    imapPort: 993,
    smtpHost: "smtp.northwind.co",
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
}

const ACCOUNT_A = account("acct-matrix-a", "ops@northwind.co");
const ACCOUNT_B = account("acct-matrix-b", "legal@northwind.co");

const OPENAI_ROW: ConfiguredProviderInfo = {
  accountId: ACCOUNT_A.id,
  email: ACCOUNT_A.email,
  displayName: "Work",
  colorToken: "slate",
  provider: "openai",
  model: "gpt-4o",
  baseUrl: null,
  authLevel: 1,
  isLocal: false,
  available: true,
  updatedAt: 0,
};

const OLLAMA_ROW: ConfiguredProviderInfo = {
  ...OPENAI_ROW,
  accountId: ACCOUNT_B.id,
  email: ACCOUNT_B.email,
  provider: "ollama",
  model: "llama3:8b",
  baseUrl: "http://localhost:11434",
  isLocal: true,
};

function renderPage() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <ProviderMatrixPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
  return qc;
}

/** Replace SOME commands' responses in-place; everything else hits the mock layer. */
function overrideCommands(overrides: Partial<Record<client.CommandName, unknown>>) {
  const realIpc = client.ipc;
  return vi.spyOn(client, "ipc").mockImplementation(((name: client.CommandName, args?: unknown) => {
    if (name in overrides) return Promise.resolve(overrides[name]);
    return realIpc(name, args as never);
  }) as never);
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("ProviderMatrixPage — grid rendering", () => {
  it("renders one cell per account × capability in fine mode", async () => {
    overrideCommands({ list_accounts: [ACCOUNT_A, ACCOUNT_B] });
    renderPage();

    const capabilities = ["Draft Reply", "Risk Check", "Summarize", "Style Profile"];
    for (const capability of capabilities) {
      expect(
        await screen.findByRole("button", { name: `${capability} · ${ACCOUNT_A.email}` }),
      ).toBeInTheDocument();
      expect(
        await screen.findByRole("button", { name: `${capability} · ${ACCOUNT_B.email}` }),
      ).toBeInTheDocument();
    }
  });

  it("collapses to a single All Accounts column in simplified mode", async () => {
    overrideCommands({ list_accounts: [ACCOUNT_A, ACCOUNT_B] });
    renderPage();
    await screen.findByRole("button", { name: `Draft Reply · ${ACCOUNT_A.email}` });

    fireEvent.click(screen.getByRole("button", { name: "Simplified" }));

    expect(screen.getByRole("button", { name: "Draft Reply · All Accounts" })).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /· All Accounts$/ })).toHaveLength(4);
    expect(
      screen.queryByRole("button", { name: `Draft Reply · ${ACCOUNT_A.email}` }),
    ).not.toBeInTheDocument();
    // The overwrite warning accompanies the shared column (F_F4 §5).
    expect(
      screen.getByText(
        "Simplified view applies one shared configuration — saving a cell overwrites all account-specific settings.",
      ),
    ).toBeInTheDocument();
  });
});

describe("MatrixCell — popover editor validation", () => {
  it("caps the backup chain at two entries", async () => {
    overrideCommands({
      list_accounts: [ACCOUNT_A],
      list_configured_providers: [OPENAI_ROW, OLLAMA_ROW],
    });
    renderPage();

    fireEvent.click(
      await screen.findByRole("button", { name: `Draft Reply · ${ACCOUNT_A.email}` }),
    );
    fireEvent.change(screen.getByLabelText("Primary provider"), { target: { value: "openai" } });

    const addBackup = screen.getByRole("button", { name: "Add Backup" });
    fireEvent.click(addBackup);
    fireEvent.click(addBackup);
    expect(screen.getByLabelText("Backup 1 provider")).toBeInTheDocument();
    expect(screen.getByLabelText("Backup 2 provider")).toBeInTheDocument();
    // A third backup is blocked (F_F4 §6 — chain length ≤ 2).
    expect(addBackup).toBeDisabled();
    expect(screen.getByText("At most two backups are allowed.")).toBeInTheDocument();
  });

  it("blocks saving when a backup repeats the primary provider", async () => {
    overrideCommands({
      list_accounts: [ACCOUNT_A],
      list_configured_providers: [OPENAI_ROW, OLLAMA_ROW],
    });
    renderPage();

    fireEvent.click(
      await screen.findByRole("button", { name: `Draft Reply · ${ACCOUNT_A.email}` }),
    );
    fireEvent.change(screen.getByLabelText("Primary provider"), { target: { value: "openai" } });
    fireEvent.click(screen.getByRole("button", { name: "Add Backup" }));
    fireEvent.change(screen.getByLabelText("Backup 1 provider"), { target: { value: "openai" } });

    expect(screen.getByRole("alert")).toHaveTextContent(
      "A backup must use a different provider than the primary.",
    );
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();

    // Switching the backup to a different provider clears the block.
    fireEvent.change(screen.getByLabelText("Backup 1 provider"), { target: { value: "ollama" } });
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save" })).not.toBeDisabled();
  });
});

describe("MatrixCell — save and warnings", () => {
  it("renders returned advisory warnings without blocking the save", async () => {
    const warnings: MatrixWarning[] = [
      {
        capability: "RiskReason",
        code: "high_cost_cloud",
        message: "Sensitivity checks run on every inbound mail.",
      },
    ];
    overrideCommands({
      list_accounts: [ACCOUNT_A],
      list_configured_providers: [OPENAI_ROW],
      update_provider_matrix: warnings,
    });
    renderPage();

    const cellButton = await screen.findByRole("button", {
      name: `Risk Check · ${ACCOUNT_A.email}`,
    });
    fireEvent.click(cellButton);
    fireEvent.change(screen.getByLabelText("Primary provider"), { target: { value: "openai" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    // The save succeeded; the warning shows as a non-blocking amber notice.
    await waitFor(() =>
      expect(
        screen.getByText(
          "Risk checks run on every inbound mail; a cloud model here can add significant cost.",
        ),
      ).toBeInTheDocument(),
    );
    expect(cellButton.className).toContain("border-s-amber");
    expect(screen.getByText("Advisory Notices")).toBeInTheDocument();
  });
});

describe("MatrixToolbar — batch operations", () => {
  it("switches all Risk Check cells to the configured local provider in one batch", async () => {
    const spy = overrideCommands({
      list_accounts: [ACCOUNT_A, ACCOUNT_B],
      list_configured_providers: [OPENAI_ROW, OLLAMA_ROW],
    });
    renderPage();
    await screen.findByRole("button", { name: `Draft Reply · ${ACCOUNT_A.email}` });

    fireEvent.click(screen.getByRole("button", { name: "Switch All Risk Checks to Local" }));

    await waitFor(() => {
      const call = spy.mock.calls.find((c) => c[0] === "batch_update_provider_matrix");
      expect(call).toBeDefined();
      const args = call?.[1] as { updates: BatchMatrixUpdate[] };
      expect(args.updates).toHaveLength(2);
      expect(args.updates.map((u) => u.accountId)).toEqual([ACCOUNT_A.id, ACCOUNT_B.id]);
      for (const update of args.updates) {
        expect(update.capability).toBe("RiskReason");
        expect(update.cell.primary.provider).toBe("ollama");
        expect(update.cell.primary.model).toBe("llama3:8b");
        expect(update.cell.backups).toEqual([]);
      }
    });
  });

  it("shows the no-local-provider notice when only cloud providers exist", async () => {
    const spy = overrideCommands({
      list_accounts: [ACCOUNT_A],
      list_configured_providers: [OPENAI_ROW],
    });
    renderPage();
    await screen.findByRole("button", { name: `Draft Reply · ${ACCOUNT_A.email}` });

    fireEvent.click(screen.getByRole("button", { name: "Switch All Risk Checks to Local" }));

    expect(await screen.findByText("No local provider configured.")).toBeInTheDocument();
    expect(spy.mock.calls.find((c) => c[0] === "batch_update_provider_matrix")).toBeUndefined();
  });
});

describe("ProviderMatrixPage — reset flow", () => {
  it("resets every account's matrix to defaults", async () => {
    const spy = overrideCommands({ list_accounts: [ACCOUNT_A, ACCOUNT_B] });
    renderPage();
    await screen.findByRole("button", { name: `Draft Reply · ${ACCOUNT_A.email}` });

    fireEvent.click(screen.getByRole("button", { name: "Reset to Defaults" }));

    await waitFor(() => {
      const resetCalls = spy.mock.calls.filter((c) => c[0] === "reset_provider_matrix_to_defaults");
      expect(resetCalls.map((c) => (c[1] as { account_id: string }).account_id)).toEqual([
        ACCOUNT_A.id,
        ACCOUNT_B.id,
      ]);
    });
    expect(await screen.findByText("Matrix reset to defaults.")).toBeInTheDocument();
  });
});
