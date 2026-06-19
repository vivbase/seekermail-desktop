// T068 tests — cloud add wizard (validation, key hygiene, verify ok/fail
// rendering), local Ollama discovery flow, the provider-list Local badge, and
// the EditAccountSheet Full-Auto confirmation intercept. Off-Tauri, `ipc()`
// resolves from the stateful mock layer in client.ts unless a test overrides
// one command in-place.
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactElement } from "react";
import type { Account } from "@shared/bindings";

import "@/i18n";
import * as client from "@/ipc/client";
import type { ConfiguredProviderInfo } from "@/ipc/aiSettings";
import AddCloudProviderSheet from "./AddCloudProviderSheet";
import AddLocalProviderSheet from "./AddLocalProviderSheet";
import ProviderListItem from "./ProviderListItem";
import EditAccountSheet from "../accounts/EditAccountSheet";

const WORK_ACCOUNT: Account = {
  id: "acct-work-1",
  email: "ops@northwind.co",
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

const LOCAL_PROVIDER_ROW: ConfiguredProviderInfo = {
  accountId: "acct-work-1",
  email: "ops@northwind.co",
  displayName: "Work",
  colorToken: "slate",
  provider: "ollama",
  model: "llama3:8b",
  baseUrl: "http://localhost:11434",
  authLevel: 1,
  isLocal: true,
  available: true,
  updatedAt: 0,
};

function renderWithProviders(ui: ReactElement) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>{ui}</MemoryRouter>
    </QueryClientProvider>,
  );
  return qc;
}

/** Replace ONE command's response in-place; everything else hits the mock layer. */
function overrideCommand(cmd: client.CommandName, response: unknown) {
  const realIpc = client.ipc;
  vi.spyOn(client, "ipc").mockImplementation(((name: client.CommandName, args?: unknown) => {
    if (name === cmd) return Promise.resolve(response);
    return realIpc(name, args as never);
  }) as never);
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("AddCloudProviderSheet — field validation", () => {
  it("blocks step 2 until an API key and a model are entered", () => {
    renderWithProviders(
      <AddCloudProviderSheet accounts={[WORK_ACCOUNT]} onClose={vi.fn()} onSaved={vi.fn()} />,
    );
    // Step 1 → 2.
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    // Submit with everything empty → key error first.
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(screen.getByRole("alert")).toHaveTextContent("An API key is required.");

    fireEvent.change(screen.getByLabelText("API Key"), { target: { value: "sk-test-key" } });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(screen.getByRole("alert")).toHaveTextContent("A model name is required.");
  });
});

describe("AddCloudProviderSheet — connection test rendering", () => {
  it("shows the 401 copy when the probe reports an auth rejection", async () => {
    overrideCommand("verify_ai_provider", {
      ok: false,
      modelName: null,
      errorMessage: "provider auth rejected",
    });
    renderWithProviders(
      <AddCloudProviderSheet accounts={[WORK_ACCOUNT]} onClose={vi.fn()} onSaved={vi.fn()} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    fireEvent.change(screen.getByLabelText("API Key"), { target: { value: "sk-bad-key" } });
    fireEvent.change(screen.getByLabelText("Model"), { target: { value: "claude-sonnet-4-6" } });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    fireEvent.click(screen.getByRole("button", { name: "Test Connection" }));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("Invalid API key."));
    // A failed probe cannot be advanced past.
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
  });

  it("shows the unreachable copy for a network failure", async () => {
    overrideCommand("verify_ai_provider", {
      ok: false,
      modelName: null,
      errorMessage: "provider unreachable: connection refused",
    });
    renderWithProviders(
      <AddCloudProviderSheet accounts={[WORK_ACCOUNT]} onClose={vi.fn()} onSaved={vi.fn()} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    fireEvent.change(screen.getByLabelText("API Key"), { target: { value: "sk-test-key" } });
    fireEvent.change(screen.getByLabelText("Model"), { target: { value: "claude-sonnet-4-6" } });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    fireEvent.click(screen.getByRole("button", { name: "Test Connection" }));
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent("Could not reach the provider endpoint."),
    );
  });
});

describe("AddCloudProviderSheet — model picker", () => {
  it("fetches the live model catalog and lets you select a returned model", async () => {
    // The catalog command returns an id that is NOT in the curated shortlist,
    // so its appearance proves the live fetch populated the dropdown.
    overrideCommand("list_cloud_models", ["claude-zeta-9"]);
    renderWithProviders(
      <AddCloudProviderSheet accounts={[WORK_ACCOUNT]} onClose={vi.fn()} onSaved={vi.fn()} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    fireEvent.change(screen.getByLabelText("API Key"), { target: { value: "sk-test-key" } });

    // Curated options exist before any fetch.
    const select = screen.getByLabelText("Model");
    expect(within(select).getByRole("option", { name: "claude-opus-4-8" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Load models" }));
    await screen.findByRole("option", { name: "claude-zeta-9" });

    fireEvent.change(select, { target: { value: "claude-zeta-9" } });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    // Advancing to the connection-test step means the model selection took.
    expect(screen.getByRole("button", { name: "Test Connection" })).toBeInTheDocument();
  });

  it("reveals a free-text input when Custom is chosen", () => {
    renderWithProviders(
      <AddCloudProviderSheet accounts={[WORK_ACCOUNT]} onClose={vi.fn()} onSaved={vi.fn()} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    fireEvent.change(screen.getByLabelText("API Key"), { target: { value: "sk-test-key" } });
    fireEvent.change(screen.getByLabelText("Model"), { target: { value: "__custom__" } });

    const custom = screen.getByLabelText("Custom model ID");
    fireEvent.change(custom, { target: { value: "gpt-5.5-2026-04-23" } });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(screen.getByRole("button", { name: "Test Connection" })).toBeInTheDocument();
  });

  it("prefills the vendor base URL and model shortlist when a preset is chosen", () => {
    renderWithProviders(
      <AddCloudProviderSheet accounts={[WORK_ACCOUNT]} onClose={vi.fn()} onSaved={vi.fn()} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Continue" })); // step 1 → 2

    // The provider radios stay mounted across steps; pick one of the top providers.
    fireEvent.click(screen.getByRole("radio", { name: "DeepSeek" }));

    expect((screen.getByLabelText("Base URL") as HTMLInputElement).value).toBe(
      "https://api.deepseek.com",
    );
    const select = screen.getByLabelText("Model");
    expect(within(select).getByRole("option", { name: "deepseek-v4-pro" })).toBeInTheDocument();
  });
});

describe("AddCloudProviderSheet — save", () => {
  it("sends aiApiKey through update_account_ai_settings and clears the key input", async () => {
    const ipcSpy = vi.spyOn(client, "ipc");
    const onSaved = vi.fn();
    renderWithProviders(
      <AddCloudProviderSheet accounts={[WORK_ACCOUNT]} onClose={vi.fn()} onSaved={onSaved} />,
    );

    // Step 1 (Anthropic preselected) → step 2.
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    const keyInput = screen.getByLabelText("API Key");
    fireEvent.change(keyInput, { target: { value: "sk-transient-test" } });
    fireEvent.change(screen.getByLabelText("Model"), { target: { value: "claude-sonnet-4-6" } });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    // Step 3: the default mock verifies in-band ok.
    fireEvent.click(screen.getByRole("button", { name: "Test Connection" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Continue" })).not.toBeDisabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    // Step 4: save to the (pre-selected) account.
    fireEvent.click(screen.getByRole("button", { name: "Save Provider" }));
    await waitFor(() => expect(onSaved).toHaveBeenCalled());

    const updateCall = ipcSpy.mock.calls.find((c) => c[0] === "update_account_ai_settings");
    expect(updateCall).toBeDefined();
    const args = updateCall?.[1] as {
      account_id: string;
      params: { aiApiKey: string | null; aiProvider: string; aiModel: string | null };
    };
    expect(args.account_id).toBe(WORK_ACCOUNT.id);
    expect(args.params.aiApiKey).toBe("sk-transient-test");
    expect(args.params.aiProvider).toBe("anthropic");
    expect(args.params.aiModel).toBe("claude-sonnet-4-6");
    // ADR-0004: the form copy of the key is dropped when submit starts.
    expect((keyInput as HTMLInputElement).value).toBe("");
  });
});

describe("AddLocalProviderSheet — discovery flow", () => {
  it("scans, lists models, selects one, verifies, and saves the ollama config", async () => {
    const ipcSpy = vi.spyOn(client, "ipc");
    const onSaved = vi.fn();
    renderWithProviders(
      <AddLocalProviderSheet accounts={[WORK_ACCOUNT]} onClose={vi.fn()} onSaved={onSaved} />,
    );

    // Step 1: scan finds the default endpoint (client.ts mock fixture).
    fireEvent.click(screen.getByRole("button", { name: "Scan for local AI" }));
    const endpoint = await screen.findByRole("radio", { name: "http://localhost:11434" });
    fireEvent.click(endpoint);
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    // Step 2: load models from the daemon, pick one.
    fireEvent.click(screen.getByRole("button", { name: "Load Models" }));
    const model = await screen.findByRole("checkbox", { name: /llama3:8b/ });
    fireEvent.click(model);
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    // Step 3: in-band verify (default mock ok).
    fireEvent.click(screen.getByRole("button", { name: "Test Connection" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Continue" })).not.toBeDisabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    // Step 4: save.
    fireEvent.click(screen.getByRole("button", { name: "Save Provider" }));
    await waitFor(() => expect(onSaved).toHaveBeenCalled());

    const updateCall = ipcSpy.mock.calls.find((c) => c[0] === "update_account_ai_settings");
    const args = updateCall?.[1] as {
      account_id: string;
      params: { aiProvider: string; aiModel: string | null; aiBaseUrl: string | null };
    };
    expect(args.account_id).toBe(WORK_ACCOUNT.id);
    expect(args.params.aiProvider).toBe("ollama");
    expect(args.params.aiModel).toBe("llama3:8b");
    expect(args.params.aiBaseUrl).toBe("http://localhost:11434");
  });
});

describe("ProviderListItem", () => {
  it("renders the Local badge for a local provider", () => {
    renderWithProviders(
      <ul>
        <ProviderListItem provider={LOCAL_PROVIDER_ROW} onEdit={vi.fn()} onRemoved={vi.fn()} />
      </ul>,
    );
    expect(screen.getByText("🔒 Local")).toBeInTheDocument();
    expect(screen.getByText("Active")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retest" })).toBeInTheDocument();
  });

  it("omits the Local badge and Retest for a cloud provider", () => {
    renderWithProviders(
      <ul>
        <ProviderListItem
          provider={{ ...LOCAL_PROVIDER_ROW, provider: "openai", isLocal: false }}
          onEdit={vi.fn()}
          onRemoved={vi.fn()}
        />
      </ul>,
    );
    expect(screen.queryByText("🔒 Local")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Retest" })).not.toBeInTheDocument();
  });
});

describe("EditAccountSheet — AI automation level", () => {
  it("intercepts Full Auto with a confirm dialog; Cancel never saves level 3", async () => {
    const ipcSpy = vi.spyOn(client, "ipc");
    const onClose = vi.fn();
    renderWithProviders(<EditAccountSheet account={WORK_ACCOUNT} onClose={onClose} />);

    fireEvent.click(screen.getByRole("radio", { name: /Full Auto/ }));
    const dialog = screen.getByRole("alertdialog");
    expect(dialog).toBeInTheDocument();

    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /Full Auto/ })).toHaveAttribute(
      "aria-checked",
      "false",
    );

    // Saving after Cancel keeps the level unchanged → no AI-settings mirror call.
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    const commands = ipcSpy.mock.calls.map((c) => c[0]);
    expect(commands).not.toContain("update_account_ai_settings");
  });

  it("applies level 3 after Confirm and mirrors it to account_ai_settings", async () => {
    const ipcSpy = vi.spyOn(client, "ipc");
    const onClose = vi.fn();
    renderWithProviders(<EditAccountSheet account={WORK_ACCOUNT} onClose={onClose} />);

    fireEvent.click(screen.getByRole("radio", { name: /Full Auto/ }));
    fireEvent.click(screen.getByRole("button", { name: "Enable Full Auto" }));
    expect(screen.getByRole("radio", { name: /Full Auto/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );

    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(onClose).toHaveBeenCalled());

    const aiCall = ipcSpy.mock.calls.find((c) => c[0] === "update_account_ai_settings");
    expect(aiCall).toBeDefined();
    const args = aiCall?.[1] as { account_id: string; params: { authLevel: number | null } };
    expect(args.account_id).toBe(WORK_ACCOUNT.id);
    expect(args.params.authLevel).toBe(3);
  });
});
