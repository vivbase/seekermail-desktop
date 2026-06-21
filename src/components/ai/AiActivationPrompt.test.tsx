// Tests for the dismissible first-run AI activation popup. Two concerns:
// (1) it surfaces only when an account has no configured provider, and closing
// it ("Maybe later") dismisses it for the session and unmounts it; (2) the
// "ready" step is a real reply-mode chooser that offers ONLY Semi-Auto (default)
// and Manual Only — Full Auto is withheld at first run (it stays locked until an
// account has >= 50 approved drafts, F_E3 §4.1) — and activating writes the
// picked tier (Semi-Auto = AuthLevel 2, Manual Only = AuthLevel 1).
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactElement } from "react";

import "@/i18n";
import { useConfiguredProviders, useUpdateAiSettings } from "@/ipc/queries/aiProviders";
import { useAccounts } from "@/ipc/queries/accounts";
import { useActivationStore } from "@/stores/activation";
import AiActivationPrompt from "./AiActivationPrompt";

const { mockMutateAsync } = vi.hoisted(() => ({ mockMutateAsync: vi.fn() }));

vi.mock("@/ipc/queries/aiProviders", async () => {
  const actual = await vi.importActual<typeof import("@/ipc/queries/aiProviders")>(
    "@/ipc/queries/aiProviders",
  );
  return { ...actual, useConfiguredProviders: vi.fn(), useUpdateAiSettings: vi.fn() };
});

vi.mock("@/ipc/queries/accounts", async () => {
  const actual =
    await vi.importActual<typeof import("@/ipc/queries/accounts")>("@/ipc/queries/accounts");
  return { ...actual, useAccounts: vi.fn() };
});

// Stub the heavy provider wizards: clicking "save" drives the prompt into the
// "ready" reply-mode step without exercising the real provider forms.
// The stub "save" button sits outside the modal, so — like the real sheet — it
// must stop propagation; otherwise the click bubbles to the overlay's close
// handler and unmounts the dialog before the "ready" step can render.
vi.mock("@/routes/settings/ai/AddCloudProviderSheet", () => ({
  default: (props: { onSaved: () => void }) => (
    <button
      type="button"
      data-testid="cloud-save"
      onClick={(e) => {
        e.stopPropagation();
        props.onSaved();
      }}
    >
      save provider
    </button>
  ),
}));
vi.mock("@/routes/settings/ai/AddLocalProviderSheet", () => ({
  default: (props: { onSaved: () => void }) => (
    <button
      type="button"
      data-testid="local-save"
      onClick={(e) => {
        e.stopPropagation();
        props.onSaved();
      }}
    >
      save local
    </button>
  ),
}));

function mockProviders(data: { provider: string; accountId: string }[]) {
  vi.mocked(useConfiguredProviders).mockReturnValue({
    data,
    isLoading: false,
  } as unknown as ReturnType<typeof useConfiguredProviders>);
}

function mockAccounts(data: { id: string }[]) {
  vi.mocked(useAccounts).mockReturnValue({
    data,
  } as unknown as ReturnType<typeof useAccounts>);
}

function renderWithProviders(ui: ReactElement) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter>{ui}</MemoryRouter>
    </QueryClientProvider>,
  );
}

// Walk from the first-run gate into the "ready" reply-mode step.
function openReadyStep() {
  renderWithProviders(<AiActivationPrompt />);
  fireEvent.click(screen.getByRole("button", { name: /add ai api key/i }));
  fireEvent.click(screen.getByTestId("cloud-save"));
}

beforeEach(() => {
  useActivationStore.setState({ dismissed: false });
  vi.mocked(useConfiguredProviders).mockReset();
  vi.mocked(useUpdateAiSettings).mockReset();
  vi.mocked(useAccounts).mockReset();
  mockMutateAsync.mockReset().mockResolvedValue(undefined);
  vi.mocked(useUpdateAiSettings).mockReturnValue({
    mutateAsync: mockMutateAsync,
  } as unknown as ReturnType<typeof useUpdateAiSettings>);
  mockAccounts([{ id: "a1" }]);
});

describe("AiActivationPrompt", () => {
  it("does not surface once a provider is configured", () => {
    mockProviders([{ provider: "openai", accountId: "a1" }]);
    renderWithProviders(<AiActivationPrompt />);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("surfaces the optional nudge when no provider is configured", () => {
    mockProviders([{ provider: "none", accountId: "a1" }]);
    renderWithProviders(<AiActivationPrompt />);
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: /bring your agents online/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /add ai api key/i })).toBeInTheDocument();
  });

  it("can be closed directly — dismisses for the session and unmounts", () => {
    mockProviders([{ provider: "none", accountId: "a1" }]);
    renderWithProviders(<AiActivationPrompt />);
    fireEvent.click(screen.getByRole("button", { name: /maybe later/i }));
    expect(useActivationStore.getState().dismissed).toBe(true);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("offers only Semi-Auto and Manual Only — Full Auto is withheld at first run", () => {
    mockProviders([{ provider: "none", accountId: "a1" }]);
    openReadyStep();
    expect(screen.getByRole("radio", { name: /semi-auto/i })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /manual only/i })).toBeInTheDocument();
    expect(screen.queryByText(/full auto/i)).not.toBeInTheDocument();
  });

  it("activates Semi-Auto (AuthLevel 2) by default", async () => {
    mockProviders([{ provider: "none", accountId: "a1" }]);
    openReadyStep();
    fireEvent.click(screen.getByRole("button", { name: /activate in semi-auto/i }));
    await waitFor(() =>
      expect(mockMutateAsync).toHaveBeenCalledWith(
        expect.objectContaining({
          accountId: "a1",
          params: expect.objectContaining({ authLevel: 2 }),
        }),
      ),
    );
  });

  it("activates Manual Only (AuthLevel 1) when picked", async () => {
    mockProviders([{ provider: "none", accountId: "a1" }]);
    openReadyStep();
    fireEvent.click(screen.getByRole("radio", { name: /manual only/i }));
    fireEvent.click(screen.getByRole("button", { name: /activate in manual only/i }));
    await waitFor(() =>
      expect(mockMutateAsync).toHaveBeenCalledWith(
        expect.objectContaining({
          accountId: "a1",
          params: expect.objectContaining({ authLevel: 1 }),
        }),
      ),
    );
  });
});
