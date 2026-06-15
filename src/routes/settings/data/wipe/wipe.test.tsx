// T053 acceptance: the typed-DELETE guard — Confirm stays disabled until the
// input matches "DELETE" exactly (case-sensitive).
import { describe, it, expect } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";

import "@/i18n";
import WipeWizard from "./index";

function renderPage() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <WipeWizard />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

async function reachConfirmStep() {
  // Step 1: pick the mock account, preview.
  const checkbox = await screen.findByRole("checkbox");
  fireEvent.click(checkbox);
  fireEvent.click(screen.getByRole("button", { name: "Preview impact" }));
  // Step 2: continue past the impact preview.
  await waitFor(() => screen.getByRole("button", { name: "Continue" }));
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  await waitFor(() => screen.getByLabelText("Confirmation text"));
}

describe("WipeWizard (T053)", () => {
  it("keeps Confirm disabled until DELETE is typed exactly", async () => {
    renderPage();
    await reachConfirmStep();

    const confirm = screen.getByRole("button", { name: "Clear data" });
    const input = screen.getByLabelText("Confirmation text");

    expect(confirm).toBeDisabled();

    fireEvent.change(input, { target: { value: "delete" } });
    expect(confirm).toBeDisabled();

    fireEvent.change(input, { target: { value: "DELETE!" } });
    expect(confirm).toBeDisabled();

    fireEvent.change(input, { target: { value: "DELETE" } });
    expect(confirm).toBeEnabled();
  });

  it("shows the impact preview numbers from the backend", async () => {
    renderPage();
    const checkbox = await screen.findByRole("checkbox");
    fireEvent.click(checkbox);
    fireEvent.click(screen.getByRole("button", { name: "Preview impact" }));
    // Mock preview: 4800 mails / 120 attachments.
    expect(await screen.findByText("4800")).toBeInTheDocument();
    expect(screen.getByText("120")).toBeInTheDocument();
  });
});
