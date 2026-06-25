// WB-17: mount once per window (in WorkbenchShell). Answers cross-window singleton queries from
// the local tab set — if this window currently shows the queried page, it focuses itself and
// claims it so the asking window stands down.
import { useEffect } from "react";

import { startSingletonResponder } from "@/ipc/singletonBroadcast";
import { useWorkbench } from "@/stores/workbench";

export function useSingletonResponder(): void {
  useEffect(() => {
    let active = true;
    let dispose: (() => void) | undefined;
    void startSingletonResponder((page) =>
      useWorkbench.getState().tabs.some((tb) => tb.route.page === page),
    ).then((un) => {
      if (active) dispose = un;
      else un();
    });
    return () => {
      active = false;
      dispose?.();
    };
  }, []);
}
