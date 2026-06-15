// T089 — ExportButton: invokes export_ai_decisions with the chosen format,
// success toast shows the filename only, FS_DISK_FULL gets dedicated copy.
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import "@/i18n";
import * as client from "@/ipc/client";
import { useToastStore } from "@/components/ui/Toast";
import { EMPTY_AUDIT_FILTERS } from "@/ipc/queries/audit";
import { ExportButton } from "./ExportButton";

function renderButton() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <ExportButton
        filters={{ ...EMPTY_AUDIT_FILTERS }}
        defaultSinceUnix={100}
        defaultUntilUnix={200}
      />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  useToastStore.setState({ toasts: [] });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("ExportButton", () => {
  it("invokes export_ai_decisions with format csv and toasts the filename only", async () => {
    const spy = vi
      .spyOn(client, "ipc")
      .mockResolvedValue("demo/exports/ai_decisions_2026-06-13.csv");
    renderButton();

    fireEvent.click(screen.getByRole("button", { name: "Export" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Export as CSV" }));

    await waitFor(() =>
      expect(spy).toHaveBeenCalledWith("export_ai_decisions", {
        params: { accountId: null, sinceUnix: 100, untilUnix: 200, format: "csv" },
      }),
    );
    await waitFor(() => {
      const messages = useToastStore.getState().toasts.map((t) => t.message);
      expect(messages.some((m) => m.includes("ai_decisions_2026-06-13.csv"))).toBe(true);
      // The directory part of the path must never surface.
      expect(messages.some((m) => m.includes("demo/exports"))).toBe(false);
    });
  });

  it("invokes with format json from the JSON menu item", async () => {
    const spy = vi.spyOn(client, "ipc").mockResolvedValue("x/y/out.json");
    renderButton();

    fireEvent.click(screen.getByRole("button", { name: "Export" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Export as JSON" }));

    await waitFor(() =>
      expect(spy).toHaveBeenCalledWith(
        "export_ai_decisions",
        expect.objectContaining({ params: expect.objectContaining({ format: "json" }) }),
      ),
    );
  });

  it("shows the disk-full message on FS_DISK_FULL", async () => {
    vi.spyOn(client, "ipc").mockRejectedValue({
      code: "FS_DISK_FULL",
      message: "No space left on device.",
      detail: null,
    });
    renderButton();

    fireEvent.click(screen.getByRole("button", { name: "Export" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Export as CSV" }));

    await waitFor(() =>
      expect(
        useToastStore.getState().toasts.some((t) => t.message === "Export failed: disk full"),
      ).toBe(true),
    );
  });
});
