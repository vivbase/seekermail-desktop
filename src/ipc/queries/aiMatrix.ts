// TanStack Query hooks for the F4 provider-matrix surface (T066, dev/02
// §Module H). Components consume these, never `ipc()` or `invoke` directly
// (07 §6). `update_provider_matrix` resolves with the backend's advisory
// warnings (F_F4 §4.5) — the save has already succeeded when they arrive, so
// callers render them as non-blocking hints.
import { useMutation, useQueries, useQueryClient } from "@tanstack/react-query";

import { ipc } from "../client";
import type { BatchMatrixUpdate, CapabilityMatrix } from "../aiMatrix";

export const aiMatrixKeys = {
  all: ["providerMatrix"] as const,
  account: (accountId: string) => ["providerMatrix", accountId] as const,
};

/**
 * One matrix query per account, in input order — the matrix page joins the
 * columns itself. An account with a `NULL` matrix column gets the computed
 * defaults back, never an error.
 */
export function useProviderMatrices(accountIds: string[]) {
  return useQueries({
    queries: accountIds.map((accountId) => ({
      queryKey: aiMatrixKeys.account(accountId),
      queryFn: () => ipc("get_provider_matrix", { account_id: accountId }),
    })),
  });
}

/** Replace one account's matrix; resolves with the advisory warnings. */
export function useUpdateProviderMatrix() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { accountId: string; matrix: CapabilityMatrix }) =>
      ipc("update_provider_matrix", { account_id: vars.accountId, matrix: vars.matrix }),
    onSuccess: (_warnings, vars) =>
      void qc.invalidateQueries({ queryKey: aiMatrixKeys.account(vars.accountId) }),
  });
}

/** Reset one account's matrix to the computed defaults (F_F4 §4.1). */
export function useResetProviderMatrix() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (accountId: string) =>
      ipc("reset_provider_matrix_to_defaults", { account_id: accountId }),
    onSuccess: (matrix, accountId) => {
      qc.setQueryData(aiMatrixKeys.account(accountId), matrix);
    },
  });
}

/** Apply a batch of cell updates across accounts/capabilities (F_F4 §4.3). */
export function useBatchUpdateProviderMatrix() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (updates: BatchMatrixUpdate[]) => ipc("batch_update_provider_matrix", { updates }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: aiMatrixKeys.all }),
  });
}
