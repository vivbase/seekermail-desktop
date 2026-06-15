// TanStack Query hooks for system commands (one file per backend module, 07 §6).
// Example hook proving the TanStack Query → ipc() → backend chain (T007).
import { useQuery } from "@tanstack/react-query";

import { ipc } from "../client";

export const systemKeys = {
  ping: ["ping"] as const,
};

/** Liveness probe used by the shell to confirm the IPC bridge is up. */
export function usePing() {
  return useQuery({
    queryKey: systemKeys.ping,
    queryFn: () => ipc("ping"),
    staleTime: 30_000,
  });
}
