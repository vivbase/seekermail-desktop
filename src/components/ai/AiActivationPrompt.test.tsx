// Tests for the dismissible first-run AI activation popup: it surfaces when an
// account has no configured provider, and configuring AI is optional — closing
// it (here via "Maybe later") dismisses it for the session and unmounts it,
// leaving the user in the app. Off-Tauri, account/update queries resolve from the
// stateful ipc mock layer; the provider query is mocked to control the gate.
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactElement } from "react";

import "@/i18n";
import { useConfiguredProviders } from "@/ipc/queries/aiProviders";
import { useActivationStore } from "@/stores/activation";
import AiActivationPrompt from "./AiActivationPrompt";

vi.mock("@/ipc/queries/aiProviders", async () => {
  const actual = await vi.importActual<typeof import("@/ipc/queries/aiProviders")>(
    "@/ipc/queries/aiProviders",
  );
  return { ...actual, useConfiguredProviders: vi.fn() };
});

function mockProviders(data: { provider: string; accountId: string }[]) {
  vi.mocked(useConfiguredProviders).mockReturnValue({
    data,
    isLoading: false,
  } as unknown as ReturnType<typeof useConfiguredProviders>);
}

function renderWithProviders(ui: ReactElement) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter>{ui}</MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  useActivationStore.setState({ dismissed: false });
  vi.mocked(useConfiguredProviders).mockReset();
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
});
