// TanStack Query hooks for the D1 legal analysis + Module E risk events (T071).
// Components consume these, never `ipc()` directly (07 §6).
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { ipc } from "../client";
import type { AnalyzeLegalRiskParams, ListRiskEventsParams, ResolveRiskParams } from "../legal";

/** 24 h — matches the backend analysis-cache window (F_D1 §4.5, T070 §3). */
export const LEGAL_ANALYSIS_STALE_MS = 24 * 60 * 60 * 1000;

export const riskKeys = {
  legalAnalysis: (mailId: string) => ["legalAnalysis", mailId] as const,
  events: (params: ListRiskEventsParams) => ["riskEvents", params] as const,
  allEvents: ["riskEvents"] as const,
};

/**
 * Lazily fetch the (possibly cached) D1 analysis for one mail. `enabled` is the
 * laziness gate: the Legal tab flips it on first open, so no provider call
 * happens before the user asks (T071 §6). `forceNew: false` lets the backend
 * replay its own 24 h cache; the matching frontend `staleTime` keeps reopened
 * tabs from re-invoking the command at all.
 */
export function useLegalAnalysis(mailId: string, enabled: boolean) {
  return useQuery({
    queryKey: riskKeys.legalAnalysis(mailId),
    queryFn: () => ipc("analyze_legal_risk", { params: { mailId, forceNew: false } }),
    enabled: enabled && !!mailId,
    staleTime: LEGAL_ANALYSIS_STALE_MS,
    retry: false, // AI errors surface immediately; the panel owns the retry UX
  });
}

/**
 * Imperative (re-)analysis — the "Try Again" path with `forceNew: true`. The
 * fresh verdict is written straight into the query cache so `useLegalAnalysis`
 * consumers re-render without a duplicate IPC round-trip.
 */
export function useAnalyzeLegalRisk() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (params: AnalyzeLegalRiskParams) => ipc("analyze_legal_risk", { params }),
    onSuccess: (result, vars) => {
      qc.setQueryData(riskKeys.legalAnalysis(vars.mailId), result);
      // A forced run may have produced new risk_events rows (T070 §3 step 7).
      void qc.invalidateQueries({ queryKey: riskKeys.allEvents });
    },
  });
}

/** Open risk events across all accounts — backs the T4 shell banner (T100).
 *  Event-driven: `risk:alert` invalidates `['riskEvents']` so it appears live. */
export function useOpenRiskEvents() {
  return useQuery({
    queryKey: riskKeys.events({ status: "open" }),
    queryFn: () => ipc("list_risk_events", { params: { status: "open" } }),
    staleTime: 0,
  });
}

/** Risk events filtered per dev/02 §Module E (default `status: 'open'`). */
export function useRiskEvents(params: ListRiskEventsParams) {
  return useQuery({
    queryKey: riskKeys.events(params),
    queryFn: () => ipc("list_risk_events", { params }),
    staleTime: 15_000,
  });
}

/** Resolve or dismiss one risk event; every riskEvents query refetches. */
export function useResolveRiskEvent() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (params: ResolveRiskParams) => ipc("resolve_risk_event", { params }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: riskKeys.allEvents }),
  });
}
