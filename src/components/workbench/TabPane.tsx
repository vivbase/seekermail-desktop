// Keep-alive tab pane (WB-02). Once opened, a pane is ALWAYS mounted; when inactive it is
// hidden with `display:none` (never unmounted) so its children keep their state — scroll
// position, selection, and compose buffers survive tab switches (18 §3, Model S).
import { type ReactNode } from "react";

import { TabActiveContext } from "./tabActiveContext";

export interface TabPaneProps {
  active: boolean;
  children: ReactNode;
}

/** One kept-alive pane. Visible only when `active`; otherwise mounted but `display:none`. */
export default function TabPane({ active, children }: TabPaneProps) {
  return (
    <TabActiveContext.Provider value={active}>
      <div
        className="h-full min-h-0 w-full"
        style={{ display: active ? "flex" : "none" }}
        data-tab-active={active ? "true" : "false"}
      >
        {children}
      </div>
    </TabActiveContext.Provider>
  );
}
