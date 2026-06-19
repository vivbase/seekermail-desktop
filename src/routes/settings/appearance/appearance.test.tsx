// T050 acceptance: selecting Dark adds `html.dark`; selecting Light removes it.
import { describe, it, expect } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
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

  // analysis 25: selecting a text size writes the whole-UI zoom multiplier. The
  // "Text size" and "Reading text size" rows share size words, so scope by group.
  it("applies the selected UI scale to the --ui-scale variable", async () => {
    renderPage();
    const group = within(screen.getByRole("group", { name: "Text size" }));

    fireEvent.click(group.getByRole("button", { name: "Large" }));
    await waitFor(() =>
      expect(document.documentElement.style.getPropertyValue("--ui-scale")).toBe("1.15"),
    );

    fireEvent.click(group.getByRole("button", { name: "Default" }));
    await waitFor(() =>
      expect(document.documentElement.style.getPropertyValue("--ui-scale")).toBe("1"),
    );
  });

  // analysis 25, Layer 2: the reading-size row writes the email-body multiplier
  // without touching the global --ui-scale.
  it("applies the selected reading size to the --reading-scale variable", async () => {
    renderPage();
    const group = within(screen.getByRole("group", { name: "Reading text size" }));

    fireEvent.click(group.getByRole("button", { name: "Larger" }));
    await waitFor(() =>
      expect(document.documentElement.style.getPropertyValue("--reading-scale")).toBe("1.3"),
    );
  });
});
