// TanStack Query hooks for the GTE index stats + topic breakdown (commands/gte.rs).
// Components consume these, never `ipc()` directly (07 §6). Mock-backed off-Tauri
// via src/ipc/client.ts so the GTE + Repository pages render in a plain browser.
import { useQuery } from "@tanstack/react-query";

import { ipc } from "../client";

/** Index/engine statistics — GTE status row + Repository stat strip / engine panel. */
export function useGteStats() {
  return useQuery({
    queryKey: ["gte_stats"],
    queryFn: () => ipc("get_gte_stats", undefined),
    staleTime: 30_000,
  });
}

/** Tagged-mail counts per topic — GTE "Top Topics" + Repository topic chart. */
export function useTopicBreakdown() {
  return useQuery({
    queryKey: ["topic_breakdown"],
    queryFn: () => ipc("get_topic_breakdown", undefined),
    staleTime: 60_000,
  });
}

/** Recent indexed mails as knowledge entries — GTE recent list + Repository browse. */
export function useKnowledgeEntries(accountId: string | null = null, limit = 20) {
  return useQuery({
    queryKey: ["knowledge_entries", accountId, limit],
    queryFn: () => ipc("list_knowledge_entries", { account_id: accountId, limit }),
    staleTime: 30_000,
  });
}
