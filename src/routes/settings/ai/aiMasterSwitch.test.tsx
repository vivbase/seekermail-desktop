// T067 / F_F5 §4.5 acceptance: the global AI master switch reads its state from
// the (mock) settings store, disables AI for a preset window, restores it, and
// renders the indefinite state. The switch drives the dedicated `set_ai_disabled`
// command (surfacing the previously backend-only F5 control).
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import "@/i18n";
import { ipc } from "@/ipc/client";
import AiMasterSwitchSection from "./AiMasterSwitchSection";

function renderSection() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <AiMasterSwitchSection />
    </QueryClientProvider>,
  );
}

describe("AiMasterSwitchSection (T067, F_F5 §4.5)", () => {
  beforeEach(async () => {
    // Reset to the active baseline so every test starts from a known state.
    await ipc("set_ai_disabled", { until: null });
  });

  it("shows the active state and disables AI for 24 hours", async () => {
    renderSection();
    await waitFor(() => screen.getByText("AI is active across all accounts"));

    fireEvent.click(screen.getByRole("button", { name: "Pause for 24 hours" }));

    await waitFor(() => expect(screen.getByText(/AI paused/)).toBeInTheDocument());
    expect(screen.getByRole("button", { name: "Resume AI now" })).toBeInTheDocument();
  });

  it("resumes AI from a disabled state", async () => {
    const now = Math.floor(Date.now() / 1000);
    await ipc("set_ai_disabled", { until: now + 3600 });
    renderSection();

    const resume = await waitFor(() => screen.getByRole("button", { name: "Resume AI now" }));
    fireEvent.click(resume);

    await waitFor(() => screen.getByText("AI is active across all accounts"));
    expect(screen.getByRole("button", { name: "Pause for 24 hours" })).toBeInTheDocument();
  });

  it("renders the indefinite (permanent) disabled state", async () => {
    await ipc("set_ai_disabled", { until: 4_102_444_800 });
    renderSection();
    await waitFor(() => screen.getByText("AI is disabled indefinitely"));
  });
});
