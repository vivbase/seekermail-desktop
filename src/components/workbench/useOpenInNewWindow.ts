// Open a workspace in a NEW OS window (T2 — WB-18/WB-20/WB-22). Encodes the TabSpec into a
// `?boot=` token and asks the backend (commands/workbench.rs: workbench_open_window) to spawn a
// window booted to that tab. Off-Tauri (dev/tests) the ipc mock no-ops; failures are swallowed.
import { useCallback } from "react";

import { ipc } from "@/ipc/client";
import { encodeBootToken } from "@/lib/bootToken";
import type { TabSpec } from "@/stores/workbench";

export function useOpenInNewWindow(): (spec: TabSpec) => void {
  return useCallback((spec: TabSpec) => {
    void ipc("workbench_open_window", { boot: encodeBootToken(spec), at: null }).catch(() => {
      /* window spawn unavailable (off-Tauri) or failed — no-op */
    });
  }, []);
}
