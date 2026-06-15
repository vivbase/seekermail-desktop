// T086 — E3ConfirmDialog gating: Enable is locked behind the acknowledgement
// checkbox, Esc cancels, closed dialog renders nothing.
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";

import "@/i18n";
import { E3ConfirmDialog } from "./E3ConfirmDialog";

describe("E3ConfirmDialog", () => {
  it("renders nothing while closed", () => {
    const { container } = render(
      <E3ConfirmDialog open={false} onConfirm={vi.fn()} onCancel={vi.fn()} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("disables Enable until the risk acknowledgement is checked", () => {
    const onConfirm = vi.fn();
    render(<E3ConfirmDialog open onConfirm={onConfirm} onCancel={vi.fn()} />);

    const enable = screen.getByRole("button", { name: "Enable Full Auto" });
    expect(enable).toBeDisabled();
    fireEvent.click(enable);
    expect(onConfirm).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("checkbox"));
    expect(enable).toBeEnabled();
    fireEvent.click(enable);
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it("lists the three locked risk rules", () => {
    render(<E3ConfirmDialog open onConfirm={vi.fn()} onCancel={vi.fn()} />);
    expect(screen.getByText(/money, payments, or bank details/i)).toBeInTheDocument();
    expect(screen.getByText(/include attachments/i)).toBeInTheDocument();
    expect(screen.getByText(/contacts marked important/i)).toBeInTheDocument();
  });

  it("cancels on Escape", () => {
    const onCancel = vi.fn();
    render(<E3ConfirmDialog open onConfirm={vi.fn()} onCancel={onCancel} />);
    fireEvent.keyDown(screen.getByRole("alertdialog"), { key: "Escape" });
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("re-locks the checkbox on every open", () => {
    const { rerender } = render(<E3ConfirmDialog open onConfirm={vi.fn()} onCancel={vi.fn()} />);
    fireEvent.click(screen.getByRole("checkbox"));
    expect(screen.getByRole("button", { name: "Enable Full Auto" })).toBeEnabled();

    rerender(<E3ConfirmDialog open={false} onConfirm={vi.fn()} onCancel={vi.fn()} />);
    rerender(<E3ConfirmDialog open onConfirm={vi.fn()} onCancel={vi.fn()} />);
    expect(screen.getByRole("button", { name: "Enable Full Auto" })).toBeDisabled();
  });
});
