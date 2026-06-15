// TanStack Query hooks + event-driven progress for the wipe task (T053 §3a).
// Components consume these, never `ipc()` or `invoke` directly (07 §6).
import { useCallback, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { WipeCompletePayload, WipeProgressPayload, WipeScope } from "@shared/bindings";

import { ipc } from "../client";
import { useEvent } from "../events";

export function usePreviewWipe() {
  return useMutation({
    mutationFn: (accountIds: string[]) => ipc("preview_wipe", { account_ids: accountIds }),
  });
}

export function useStartWipe() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { accountIds: string[]; scope: WipeScope }) =>
      ipc("start_wipe", { account_ids: vars.accountIds, scope: vars.scope }),
    onSuccess: () => {
      // Mail lists / accounts will be stale once the task lands.
      void qc.invalidateQueries({ queryKey: ["threads"] });
      void qc.invalidateQueries({ queryKey: ["accounts"] });
    },
  });
}

export interface WipeTaskState {
  progress: WipeProgressPayload | null;
  complete: WipeCompletePayload | null;
}

/** Subscribe to `wipe:*` for one task id. */
export function useWipeProgress(taskId: string | null): WipeTaskState {
  const [progress, setProgress] = useState<WipeProgressPayload | null>(null);
  const [complete, setComplete] = useState<WipeCompletePayload | null>(null);

  const onProgress = useCallback(
    (p: WipeProgressPayload) => {
      if (taskId && p.taskId === taskId) setProgress(p);
    },
    [taskId],
  );
  const onComplete = useCallback(
    (p: WipeCompletePayload) => {
      if (taskId && p.taskId === taskId) setComplete(p);
    },
    [taskId],
  );

  useEvent<WipeProgressPayload>("wipe:progress", onProgress);
  useEvent<WipeCompletePayload>("wipe:complete", onComplete);

  return { progress, complete };
}
