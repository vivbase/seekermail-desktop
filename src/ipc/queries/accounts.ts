// TanStack Query hooks for the account commands (T017). Components consume these,
// never `ipc()` or `invoke` directly (07 §6).
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type {
  CreateAccountParams,
  ImageAllowScope,
  Provider,
  UpdateAccountParams,
  VerifyConnectionParams,
} from "@shared/bindings";

import { ipc } from "../client";
import type { UpdateAiSettingsParams } from "../aiSettings";

export const accountKeys = {
  all: ["accounts"] as const,
  detail: (id: string) => ["accounts", id] as const,
  syncState: (id: string) => ["sync_state", id] as const,
  syncProgress: (id: string) => ["sync_progress", id] as const,
  syncError: (id: string) => ["sync_error", id] as const,
  backfill: (id: string) => ["backfill", id] as const,
  diskUsage: (id: string) => ["disk_usage", id] as const,
  aiSettings: (id: string) => ["accountAiSettings", id] as const,
};

/** All accounts (the routing gate + the settings list both read this). */
export function useAccounts() {
  return useQuery({
    queryKey: accountKeys.all,
    queryFn: () => ipc("list_accounts"),
    staleTime: 10_000,
  });
}

export function useAccount(accountId: string) {
  return useQuery({
    queryKey: accountKeys.detail(accountId),
    queryFn: () => ipc("get_account", { account_id: accountId }),
    enabled: !!accountId,
  });
}

export function useCreateAccount() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (params: CreateAccountParams) => ipc("create_account", { params }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: accountKeys.all }),
  });
}

export function useUpdateAccount() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { accountId: string; patch: UpdateAccountParams }) =>
      ipc("update_account", { account_id: vars.accountId, patch: vars.patch }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: accountKeys.all }),
  });
}

export function useDeleteAccount() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (accountId: string) => ipc("delete_account", { account_id: accountId }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: accountKeys.all }),
  });
}

export function useEnableAccount() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (accountId: string) => ipc("enable_account", { account_id: accountId }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: accountKeys.all }),
  });
}

export function useDisableAccount() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (accountId: string) => ipc("disable_account", { account_id: accountId }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: accountKeys.all }),
  });
}

/**
 * Promote an account to primary (T091). The backend swaps the flag atomically;
 * on success every accounts query refetches so the ★ marker moves in one render.
 */
export function useSetPrimaryAccount() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (accountId: string) => ipc("set_primary_account", { account_id: accountId }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: accountKeys.all }),
  });
}

/** Per-account `account_ai_settings` row — lazily fetched by each AgentCard (T073). */
export function useAccountAiSettings(accountId: string) {
  return useQuery({
    queryKey: accountKeys.aiSettings(accountId),
    queryFn: () => ipc("get_account_ai_settings", { account_id: accountId }),
    enabled: !!accountId,
  });
}

/**
 * Partial update of `account_ai_settings` (T073). `accounts.auth_level` is the
 * single source of truth; callers mirror it here AFTER `update_account` succeeds
 * so the AI engine reads a consistent value (dev/01 §account_ai_settings).
 */
export function useUpdateAccountAiSettings() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { accountId: string; params: UpdateAiSettingsParams }) =>
      ipc("update_account_ai_settings", { account_id: vars.accountId, params: vars.params }),
    onSuccess: (_data, vars) =>
      void qc.invalidateQueries({ queryKey: accountKeys.aiSettings(vars.accountId) }),
  });
}

export function useVerifyConnection() {
  return useMutation({
    mutationFn: (params: VerifyConnectionParams) => ipc("verify_account_connection", { params }),
  });
}

export function useBeginOAuth() {
  return useMutation({
    mutationFn: (vars: { provider: Provider; accountId: string }) =>
      ipc("begin_oauth_flow", { provider: vars.provider, account_id: vars.accountId }),
  });
}

export function useReauthAccount() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { accountId: string; password: string | null }) =>
      ipc("reauth_account", { account_id: vars.accountId, password: vars.password }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: accountKeys.all }),
  });
}

export function useSampleMailbox() {
  return useMutation({
    mutationFn: (accountId: string) => ipc("sample_mailbox", { account_id: accountId }),
  });
}

export function useSetKnowledgeDepth() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { accountId: string; months: number | null }) =>
      ipc("set_knowledge_depth", { account_id: vars.accountId, months: vars.months }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: accountKeys.all }),
  });
}

export function useSyncState(accountId: string) {
  return useQuery({
    queryKey: accountKeys.syncState(accountId),
    queryFn: () => ipc("get_sync_state", { account_id: accountId }),
    enabled: !!accountId,
    // Fallback polling until the event stream lands (T024 supersedes this).
    refetchInterval: 10_000,
  });
}

export function useDiskUsage(accountId: string) {
  return useQuery({
    queryKey: accountKeys.diskUsage(accountId),
    queryFn: () => ipc("get_account_disk_usage", { account_id: accountId }),
    enabled: !!accountId,
  });
}

export function useBackfillStatus(accountId: string) {
  return useQuery({
    queryKey: accountKeys.backfill(accountId),
    queryFn: () => ipc("get_backfill_status", { account_id: accountId }),
    enabled: !!accountId,
    refetchInterval: 10_000,
  });
}

export function useTriggerSync() {
  return useMutation({
    mutationFn: (accountId: string) => ipc("trigger_sync", { account_id: accountId }),
  });
}

export function usePauseBackfill() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (accountId: string) => ipc("pause_backfill", { account_id: accountId }),
    onSuccess: (_d, accountId) =>
      void qc.invalidateQueries({ queryKey: accountKeys.backfill(accountId) }),
  });
}

export function useResumeBackfill() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (accountId: string) => ipc("resume_backfill", { account_id: accountId }),
    onSuccess: (_d, accountId) =>
      void qc.invalidateQueries({ queryKey: accountKeys.backfill(accountId) }),
  });
}

// Re-export for the wizard's remote-image step convenience.
export type { ImageAllowScope };
