// TanStack Query hooks for Agent presence (T094). Components consume these, never
// `ipc()` directly (07 §6). Presence is polled every 30 s for now; once the
// Agent-IM event stream lands (T101) this can move to event-driven invalidation.
import { useQuery } from "@tanstack/react-query";

import { ipc } from "../client";

export const agentKeys = {
  statuses: ["agent_statuses"] as const,
};

/** All accounts' current Agent presence (processing / idle / offline). */
export function useAgentStatuses() {
  return useQuery({
    queryKey: agentKeys.statuses,
    queryFn: () => ipc("get_agent_statuses"),
    refetchInterval: 30_000,
    staleTime: 15_000,
  });
}
