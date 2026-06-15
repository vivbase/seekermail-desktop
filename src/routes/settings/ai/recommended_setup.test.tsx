// T064 acceptance: wizard step navigation (three cards → two tiers), the
// non-bypassable data-flow disclosure (shown before the first cloud
// authorization, skipped once confirmed, undismissable except via its two
// explicit buttons), the manual-code completion path to "Ready", and the
// failure surface with the spec's three exits (F_F3 §5).
//
// Tests run off-Tauri: `ipc()` resolves from the client mock layer, whose
// `MOCK_AI_SETUP` state persists across tests in this file — the order below
// is deliberate (disclosure-unconfirmed tests first).
import { describe, it, expect } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import "@/i18n";
import RecommendedSetupWizard from "./RecommendedSetupWizard";

function renderWizard(onClose: () => void = () => {}) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <RecommendedSetupWizard onClose={onClose} />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

/** Step 0 → Step 1 (three cards → tier cards). */
async function goToTiers() {
  fireEvent.click(screen.getByRole("button", { name: /Use the Recommended Plan/ }));
  await waitFor(() => expect(screen.getByText("Balanced")).toBeInTheDocument());
}

/** The balanced tier's Connect button (first tier card). */
function firstConnectButton(): HTMLElement {
  const buttons = screen.getAllByRole("button", { name: "Connect" });
  expect(buttons.length).toBe(2);
  return buttons[0] as HTMLElement;
}

describe("RecommendedSetupWizard (T064)", () => {
  it("renders the three entry cards and navigates Step 0 → Step 1", async () => {
    renderWizard();
    expect(screen.getByRole("button", { name: /Use the Recommended Plan/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Use My Own API Key/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Use a Local Model/ })).toBeInTheDocument();

    await goToTiers();
    expect(screen.getByText("High Quality")).toBeInTheDocument();
    // Cost estimate from the recommendation list (F_F3 §4.4).
    expect(screen.getAllByText(/month for about 200 AI replies/).length).toBe(2);
  });

  it("gates the first cloud authorization behind the non-bypassable disclosure", async () => {
    renderWizard();
    await goToTiers();

    fireEvent.click(firstConnectButton());
    const dialog = await waitFor(() => screen.getByRole("alertdialog"));
    expect(within(dialog).getByText("Where your data goes")).toBeInTheDocument();

    // Clicking the overlay must NOT dismiss it, and the flow must not advance.
    fireEvent.click(screen.getByRole("presentation"));
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
    expect(screen.queryByText(/Waiting for authorization/)).not.toBeInTheDocument();

    // Cancel backs out without recording anything — re-selecting re-shows it.
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(screen.queryByText(/Waiting for authorization/)).not.toBeInTheDocument();
    fireEvent.click(firstConnectButton());
    expect(await waitFor(() => screen.getByRole("alertdialog"))).toBeInTheDocument();

    // Only the explicit confirmation proceeds to the authorization step.
    fireEvent.click(screen.getByRole("button", { name: "I Understand" }));
    await waitFor(() => expect(screen.getByText(/Waiting for authorization/)).toBeInTheDocument());
  });

  it("skips the disclosure once confirmed and completes via the manual code path", async () => {
    renderWizard();
    await goToTiers();

    // Confirmed in the previous test (persisted mock state) → no modal now.
    fireEvent.click(firstConnectButton());
    await waitFor(() => expect(screen.getByText(/Waiting for authorization/)).toBeInTheDocument());
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();

    // Deep link did not return — paste the authorization code (F_F3 §6).
    fireEvent.change(screen.getByLabelText("Authorization code"), {
      target: { value: "grant-code-7f3a" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Submit Code" }));

    await waitFor(() => expect(screen.getByText("Ready 🎉")).toBeInTheDocument());
    // First-week conservative quota notice (F_F3 §4.6).
    expect(screen.getByText(/100 cloud calls per day/)).toBeInTheDocument();
  });

  it("shows the error surface with Retry / my-key / local-model exits on a rejected code", async () => {
    renderWizard();
    await goToTiers();
    fireEvent.click(firstConnectButton());
    await waitFor(() => expect(screen.getByText(/Waiting for authorization/)).toBeInTheDocument());

    fireEvent.change(screen.getByLabelText("Authorization code"), {
      target: { value: "invalid-code" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Submit Code" }));

    await waitFor(() => expect(screen.getByText("Setup did not complete")).toBeInTheDocument());
    expect(screen.getByText("The provider rejected the authorization code.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Use My Own API Key/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Use a Local Model/ })).toBeInTheDocument();

    // Retry restarts the authorization for the same tier.
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(screen.getByText(/Waiting for authorization/)).toBeInTheDocument());
  });
});
