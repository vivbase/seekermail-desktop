// TanStack Query hooks for the BYO-AI provider config surface (T068, dev/02
// §Module H). Components consume these, never `ipc()` or `invoke` directly
// (07 §6). The API key only ever travels inside a mutation payload on its way
// to the Keychain — no hook caches or re-exposes it (ADR-0004).
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { ipc } from "../client";
import type { UpdateAiSettingsParams, VerifyAiProviderParams } from "../aiSettings";
import { accountKeys } from "./accounts";

export const aiProviderKeys = {
  configured: ["configuredProviders"] as const,
};

/** Provider summary list — Settings → AI Providers and the F4 matrix UI. */
export function useConfiguredProviders() {
  return useQuery({
    queryKey: aiProviderKeys.configured,
    queryFn: () => ipc("list_configured_providers"),
  });
}

/** In-band connection probe: failures resolve with `ok: false`, never throw (09 §2). */
export function useVerifyProvider() {
  return useMutation({
    mutationFn: (params: VerifyAiProviderParams) => ipc("verify_ai_provider", { params }),
  });
}

/** Probe the default local endpoints (may take 2–4 s — callers show a spinner). */
export function useScanLocalProviders() {
  return useMutation({
    mutationFn: () => ipc("scan_local_providers"),
  });
}

/** Models installed on an Ollama daemon; `null` probes the default endpoint. */
export function useListOllamaModels() {
  return useMutation({
    mutationFn: (baseUrl: string | null) => ipc("list_ollama_models", { base_url: baseUrl }),
  });
}

/**
 * Save provider settings for one account. Refreshes the provider list AND the
 * per-account AI-settings row so the /agents cards stay consistent. `aiApiKey`
 * is consumed at the command boundary and written to the Keychain — it never
 * comes back in any response (ADR-0004).
 */
export function useUpdateAiSettings() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { accountId: string; params: UpdateAiSettingsParams }) =>
      ipc("update_account_ai_settings", { account_id: vars.accountId, params: vars.params }),
    onSuccess: (_data, vars) => {
      void qc.invalidateQueries({ queryKey: aiProviderKeys.configured });
      void qc.invalidateQueries({ queryKey: accountKeys.aiSettings(vars.accountId) });
    },
  });
}
