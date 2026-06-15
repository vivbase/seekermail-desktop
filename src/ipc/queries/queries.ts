// TanStack Query hooks for I3/I4 pending queries (T101 count + T099 list/answer/
// skip). Components consume these, never `ipc()` directly (07 §6).
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { ipc } from "../client";

export const pendingQueryKeys = {
  /** Prefix shared by every pending-query cache entry; events invalidate this. */
  all: ["pendingQueries"] as const,
  count: ["pendingQueries", "count"] as const,
  list: (accountId: string | null) => ["pendingQueries", "list", accountId] as const,
};

/** Number of queries awaiting a human decision (drives the TEAM nav badge). */
export function usePendingQueriesCount() {
  return useQuery({
    queryKey: pendingQueryKeys.count,
    queryFn: () => ipc("count_pending_queries"),
    staleTime: 5000,
  });
}

/** Pending decision cards for the Pending page (T099). */
export function usePendingQueries(accountId: string | null = null) {
  return useQuery({
    queryKey: pendingQueryKeys.list(accountId),
    queryFn: () => ipc("list_pending_queries", { account_id: accountId }),
    staleTime: 5000,
  });
}

/** Invalidate every pending-query surface + the channel after a mutation. */
function useInvalidateQueries() {
  const qc = useQueryClient();
  return () => {
    void qc.invalidateQueries({ queryKey: pendingQueryKeys.all });
    void qc.invalidateQueries({ queryKey: ["pending_counts"] });
    void qc.invalidateQueries({ queryKey: ["imMessages"] });
  };
}

/** Submit a human answer to a query; resumes the AI chain on the backend (T096). */
export function useAnswerQuery() {
  const invalidate = useInvalidateQueries();
  return useMutation({
    mutationFn: (vars: { id: string; answer: string }) => ipc("answer_query", vars),
    onSuccess: invalidate,
  });
}

/** Skip a query (conservative fallback applied on the backend, T096). */
export function useSkipQuery() {
  const invalidate = useInvalidateQueries();
  return useMutation({
    mutationFn: (id: string) => ipc("skip_query", { id }),
    onSuccess: invalidate,
  });
}
