// TanStack Query hook for the data-flow disclosure command (T069). Components
// consume this, never `ipc()` or `invoke` directly (07 §6).
import { useQuery } from "@tanstack/react-query";

import { ipc } from "../client";

export const dataFlowKeys = {
  aiRouting: ["data-flow-ai"] as const,
};

/**
 * Per-account effective AI routing (provider + real endpoint + local/cloud
 * classification) plus the 24h `ai_decisions` activity summary, for the
 * Settings → Data → Data Flow panel (dev/06 §8, ADR-0004).
 */
export function useDataFlowAiRouting() {
  return useQuery({
    queryKey: dataFlowKeys.aiRouting,
    queryFn: () => ipc("get_data_flow_ai_routing"),
  });
}
