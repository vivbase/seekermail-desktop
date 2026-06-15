// TanStack Query hooks for the F3 recommended-provider setup (T064).
// Components consume these, never `ipc()` or `invoke` directly (07 §6).
import { useEffect } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { ipc, isTauri } from "../client";
import type { RecommendedOAuthCallback, RecommendedTier } from "../recommended";

export const aiSetupKeys = {
  recommended: ["aiSetup", "recommendedProviders"] as const,
  status: ["aiSetup", "status"] as const,
};

/** Tauri event the deep-link handler emits for a recommended OAuth callback. */
export const OAUTH_CALLBACK_EVENT = "oauth:callback";

/** The built-in recommendation tiers (static config — cache aggressively). */
export function useRecommendedProviders() {
  return useQuery({
    queryKey: aiSetupKeys.recommended,
    queryFn: () => ipc("get_recommended_providers"),
    staleTime: Infinity,
  });
}

/** Disclosure / conservative-quota / first-auth snapshot for the wizard. */
export function useAiSetupStatus() {
  return useQuery({
    queryKey: aiSetupKeys.status,
    queryFn: () => ipc("get_ai_setup_status"),
  });
}

/** Record the data-flow disclosure confirmation (dev/06 §8 — once, audited). */
export function useConfirmAiDisclosure() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => ipc("confirm_ai_disclosure"),
    onSuccess: (status) => qc.setQueryData(aiSetupKeys.status, status),
  });
}

/** Lift the first-week conservative quota early (F_F3 §4.6, settings page). */
export function useClearConservativeQuota() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => ipc("clear_conservative_quota"),
    onSuccess: () => void qc.invalidateQueries({ queryKey: aiSetupKeys.status }),
  });
}

/** Start a grant: the backend opens the system browser; keep `state` around. */
export function useBeginRecommendedOAuth() {
  return useMutation({
    mutationFn: (tier: RecommendedTier) => ipc("begin_recommended_oauth", { tier }),
  });
}

/** Finish a grant from the deep-link callback or the manual code paste. */
export function useCompleteRecommendedOAuth() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { stateNonce: string; code: string }) =>
      ipc("complete_recommended_oauth", {
        state_nonce: vars.stateNonce,
        code: vars.code,
      }),
    onSuccess: () => {
      // The completion rewrote every account's AI settings + the F4 matrix.
      void qc.invalidateQueries({ queryKey: aiSetupKeys.status });
      void qc.invalidateQueries({ queryKey: ["accountAiSettings"] });
      void qc.invalidateQueries({ queryKey: ["accounts"] });
    },
  });
}

/** Disconnect a recommended tier (F_F3 §4.5). */
export function useRevokeRecommendedProvider() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (tier: RecommendedTier) => ipc("revoke_recommended_provider", { tier }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: aiSetupKeys.status });
      void qc.invalidateQueries({ queryKey: ["accountAiSettings"] });
    },
  });
}

/**
 * Subscribe to the recommended OAuth deep-link callback while mounted. The
 * deep-link handler validates nothing — the wizard forwards `code` + `state`
 * to `complete_recommended_oauth`, where the CSRF check lives. No-op outside
 * Tauri (dev browser / vitest), where the manual code paste is the path.
 */
export function useOAuthCallbackListener(onCallback: (payload: RecommendedOAuthCallback) => void) {
  useEffect(() => {
    if (!isTauri()) return undefined;
    const pending: Promise<UnlistenFn> = listen<RecommendedOAuthCallback>(
      OAUTH_CALLBACK_EVENT,
      (event) => onCallback(event.payload),
    );
    return () => {
      void pending.then((unlisten) => unlisten());
    };
    // The handler is intentionally captured fresh per mount; wizard callers
    // pass a stable callback from useCallback.
  }, [onCallback]);
}
