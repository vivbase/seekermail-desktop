import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";

import "@/i18n";
import { useWorkbench } from "@/stores/workbench";
import WorkbenchContextMenu from "./WorkbenchContextMenu";
import { openSpecAttr } from "@/lib/openSpec";

describe("WorkbenchContextMenu (WB-19)", () => {
  beforeEach(() => {
    useWorkbench.setState({ tabs: [], activeTabId: null, recentlyClosed: [] });
  });

  it("right-click on a marked element opens a new tab for it", () => {
    render(
      <div
        data-testid="row"
        {...openSpecAttr({ route: { page: "thread", params: { mailId: "42" } } })}
      >
        Row
        <WorkbenchContextMenu />
      </div>,
    );
    fireEvent.contextMenu(screen.getByTestId("row"));
    fireEvent.click(screen.getByRole("menuitem", { name: "Open in new tab" }));
    const tabs = useWorkbench.getState().tabs;
    expect(tabs).toHaveLength(1);
    expect(tabs[0]?.route).toEqual({ page: "thread", params: { mailId: "42" } });
  });

  it("ignores right-click that is not on a marked element", () => {
    render(
      <div>
        <span data-testid="plain">plain</span>
        <WorkbenchContextMenu />
      </div>,
    );
    fireEvent.contextMenu(screen.getByTestId("plain"));
    expect(screen.queryByRole("menuitem")).not.toBeInTheDocument();
  });
});
