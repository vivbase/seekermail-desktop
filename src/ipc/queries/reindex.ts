// TanStack Query hooks for the reindex task (T053 §3b). Progress arrives on the
// existing `gte:*` stream; the completion report is persisted by the backend at
// `app_settings.gte.last_reindex_report` and read back through `get_setting`.
import { useCallback, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { GteFinishedPayload, GteProgressPayload } from "@shared/bindings";

import { ipc } from "../client";
import { useEvent } from "../events";

/** Backend-owned settings key holding the last completion report (T053). */
export const REINDEX_REPORT_KEY = "gte.last_reindex_report";

export interface ReindexReport {
  processed: number;
  verifiedSample: number;
  verifyErrors: number;
  elapsedMs: number;
  finishedAt: number;
}

export function useStartReindex() {
  return useMutation({
    mutationFn: (accountId: string | null) => ipc("start_reindex", { account_id: accountId }),
  });
}

export function useCancelReindex() {
  return useMutation({
    mutationFn: (taskId: string) => ipc("cancel_reindex", { task_id: taskId }),
  });
}

/** Last completion report (null until a reindex has finished once). */
export function useReindexReport() {
  const qc = useQueryClient();
  const query = useQuery({
    queryKey: ["appSetting", REINDEX_REPORT_KEY],
    queryFn: async (): Promise<ReindexReport | null> => {
      const raw = await ipc("get_setting", { key: REINDEX_REPORT_KEY });
      if (raw === null) return null;
      try {
        return JSON.parse(raw) as ReindexReport;
      } catch {
        return null;
      }
    },
  });
  const refresh = useCallback(
    () => void qc.invalidateQueries({ queryKey: ["appSetting", REINDEX_REPORT_KEY] }),
    [qc],
  );
  return { ...query, refresh };
}

export interface ReindexProgressState {
  progress: GteProgressPayload | null;
  finished: GteFinishedPayload | null;
}

/** Subscribe to the gte stream while a reindex runs. */
export function useReindexProgress(active: boolean): ReindexProgressState {
  const [progress, setProgress] = useState<GteProgressPayload | null>(null);
  const [finished, setFinished] = useState<GteFinishedPayload | null>(null);

  const onProgress = useCallback(
    (p: GteProgressPayload) => {
      if (active) setProgress(p);
    },
    [active],
  );
  const onFinished = useCallback(
    (p: GteFinishedPayload) => {
      if (active) setFinished(p);
    },
    [active],
  );

  useEvent<GteProgressPayload>("gte:progress", onProgress);
  useEvent<GteFinishedPayload>("gte:finished", onFinished);

  return { progress, finished };
}

// ── Sync range (T053 §3c) ─────────────────────────────────────────────────────

export function usePreviewSyncRange() {
  return useMutation({
    mutationFn: (vars: { accountId: string; months: number | null }) =>
      ipc("preview_sync_range", { account_id: vars.accountId, months: vars.months }),
  });
}

export function useUpdateSyncRange() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { accountId: string; months: number | null }) =>
      ipc("update_sync_range", { account_id: vars.accountId, months: vars.months }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["accounts"] });
      void qc.invalidateQueries({ queryKey: ["threads"] });
    },
  });
}
