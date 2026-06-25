import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";

import "@/i18n";
import { useWorkbench } from "@/stores/workbench";
import TabSwitcher from "./TabSwitcher";

function seed() {
  const a = useWorkbench.getState().openTab({ route: { page: "inbox" }, accountId: "x" }); // "Inbox"
  const b = useWorkbench.getState().openTab({ route: { page: "search" } }); // "Search"
  return { a, b };
}

describe("TabSwitcher (WB-08)", () => {
  beforeEach(() => {
    useWorkbench.setState({ tabs: [], activeTabId: null, recentlyClosed: [] });
  });

  it("renders nothing when closed", () => {
    seed();
    const { container } = render(<TabSwitcher open={false} onClose={() => {}} />);
    expect(container.firstChild).toBeNull();
  });

  it("lists open tabs and filters by title", () => {
    seed();
    render(<TabSwitcher open onClose={() => {}} />);
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByText("Inbox")).toBeInTheDocument();
    expect(screen.getByText("Search")).toBeInTheDocument();

    fireEvent.change(screen.getByRole("textbox"), { target: { value: "sea" } });
    expect(screen.queryByText("Inbox")).not.toBeInTheDocument();
    expect(screen.getByText("Search")).toBeInTheDocument();
  });

  it("jumps to a tab on click and closes", () => {
    const { a } = seed();
    const onClose = vi.fn();
    render(<TabSwitcher open onClose={onClose} />);
    fireEvent.click(screen.getByText("Inbox"));
    expect(useWorkbench.getState().activeTabId).toBe(a);
    expect(onClose).toHaveBeenCalled();
  });

  it("Enter activates the highlighted result", () => {
    const { a } = seed(); // active = b (Search); first item is Inbox (a)
    const onClose = vi.fn();
    render(<TabSwitcher open onClose={onClose} />);
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Enter" });
    expect(useWorkbench.getState().activeTabId).toBe(a);
    expect(onClose).toHaveBeenCalled();
  });

  it("Esc closes the palette", () => {
    seed();
    const onClose = vi.fn();
    render(<TabSwitcher open onClose={onClose} />);
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });
});
