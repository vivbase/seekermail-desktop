// T050 acceptance: selecting Dark adds `html.dark`; selecting Light removes it.
import { describe, it, expect } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import "@/i18n";
import AppearanceSettings from "./index";

function renderPage() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <AppearanceSettings />
    </QueryClientProvider>,
  );
}

describe("AppearanceSettings (T050)", () => {
  it("applies and removes the dark class as the theme toggles", async () => {
    renderPage();

    fireEvent.click(screen.getByRole("button", { name: "Dark" }));
    await waitFor(() => expect(document.documentElement.classList.contains("dark")).toBe(true));

    fireEvent.click(screen.getByRole("button", { name: "Light" }));
    await waitFor(() => expect(document.documentElement.classList.contains("dark")).toBe(false));
  });

  it("marks the active option with aria-pressed", async () => {
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: "Dark" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Dark" })).toHaveAttribute("aria-pressed", "true"),
    );
  });
});
