// TanStack Query hook for the P-4 memory layer (commands/memory.rs). Rebuilds
// the per-thread summaries + inbox digest the agent reads as its "memory."
import { useMutation } from "@tanstack/react-query";

import { ipc } from "../client";

/** (Re)build thread summaries + the inbox digest. `accountId` null = all active
 *  accounts. Resolves to the number of summaries (re)built. Safe to repeat: the
 *  backend only touches stale or unsummarised threads. */
export function useBuildThreadSummaries() {
  return useMutation({
    mutationFn: (accountId: string | null) =>
      ipc("build_thread_summaries", { account_id: accountId, limit: null }),
  });
}
