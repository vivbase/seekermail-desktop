import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook } from "@testing-library/react";

import { useConfiguredProviders } from "@/ipc/queries/aiProviders";
import { useActivationStore } from "@/stores/activation";
import { useAiActivationGate } from "./aiActivationGate";

vi.mock("@/ipc/queries/aiProviders", () => ({
  useConfiguredProviders: vi.fn(),
}));

type ProviderRow = { provider: string; accountId: string };

function mockProviders(state: { data?: ProviderRow[]; isLoading: boolean }) {
  vi.mocked(useConfiguredProviders).mockReturnValue(
    state as unknown as ReturnType<typeof useConfiguredProviders>,
  );
}

beforeEach(() => {
  useActivationStore.setState({ dismissed: false });
  vi.mocked(useConfiguredProviders).mockReset();
});

describe("useAiActivationGate", () => {
  it("is not ready while the providers query is loading", () => {
    mockProviders({ data: undefined, isLoading: true });
    const { result } = renderHook(() => useAiActivationGate());
    expect(result.current).toEqual({ ready: false, needsActivation: false });
  });

  it("needs activation when an account has no configured provider", () => {
    mockProviders({ data: [{ provider: "none", accountId: "a1" }], isLoading: false });
    const { result } = renderHook(() => useAiActivationGate());
    expect(result.current).toEqual({ ready: true, needsActivation: true });
  });

  it("does not gate once any provider is configured", () => {
    mockProviders({ data: [{ provider: "openai", accountId: "a1" }], isLoading: false });
    const { result } = renderHook(() => useAiActivationGate());
    expect(result.current).toEqual({ ready: true, needsActivation: false });
  });

  it("stays suppressed after the user skips for this session", () => {
    mockProviders({ data: [{ provider: "none", accountId: "a1" }], isLoading: false });
    useActivationStore.setState({ dismissed: true });
    const { result } = renderHook(() => useAiActivationGate());
    expect(result.current.needsActivation).toBe(false);
  });
});
