// T051 acceptance: the two policies render from the (mock) store, selections
// persist through `apply_privacy_policy`, and Reset restores the defaults
// behind an explicit confirm dialog.
import { describe, it, expect } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import "@/i18n";
import PrivacySettings from "./index";

function renderPage() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <PrivacySettings />
    </QueryClientProvider>,
  );
}

function group(name: string) {
  return screen.getByRole("radiogroup", { name });
}

describe("PrivacySettings (T051)", () => {
  it("renders both policies and persists a tracker change", async () => {
    renderPage();
    const tracker = await waitFor(() => group("Tracking Protection"));

    fireEvent.click(within(tracker).getByRole("radio", { name: "Block All" }));
    await waitFor(() =>
      expect(within(tracker).getByRole("radio", { name: "Block All" })).toHaveAttribute(
        "aria-checked",
        "true",
      ),
    );
  });

  it("persists an image-policy change", async () => {
    renderPage();
    const images = await waitFor(() => group("Remote Image Loading"));

    fireEvent.click(within(images).getByRole("radio", { name: "Allow All" }));
    await waitFor(() =>
      expect(within(images).getByRole("radio", { name: "Allow All" })).toHaveAttribute(
        "aria-checked",
        "true",
      ),
    );
  });

  it("resets to defaults behind a confirm dialog", async () => {
    renderPage();
    await waitFor(() => group("Tracking Protection"));

    fireEvent.click(screen.getByRole("button", { name: "Reset to Defaults" }));
    const dialog = screen.getByRole("alertdialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Reset" }));

    await waitFor(() => {
      const tracker = group("Tracking Protection");
      expect(within(tracker).getByRole("radio", { name: "Block Known Trackers" })).toHaveAttribute(
        "aria-checked",
        "true",
      );
    });
    await waitFor(() => {
      const images = group("Remote Image Loading");
      expect(within(images).getByRole("radio", { name: "Block All" })).toHaveAttribute(
        "aria-checked",
        "true",
      );
    });
  });
});
