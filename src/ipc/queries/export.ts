// TanStack Query hooks + event-driven progress for the export task (T052).
// Components consume these, never `ipc()` or `invoke` directly (07 §6). Long
// tasks return a `task_id` immediately and stream `export:*` events — progress
// is event-driven, never polled (07 §6).
import { useCallback, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import type {
  ExportCompletePayload,
  ExportErrorPayload,
  ExportProgressPayload,
  StartExportParams,
} from "@shared/bindings";

import { ipc } from "../client";
import { useEvent } from "../events";

export function useStartExport() {
  return useMutation({
    mutationFn: (params: StartExportParams) => ipc("start_export", { params }),
  });
}

export function useCancelExport() {
  return useMutation({
    mutationFn: (taskId: string) => ipc("cancel_export", { task_id: taskId }),
  });
}

export function useOpenExportOutput() {
  return useMutation({
    mutationFn: (taskId: string) => ipc("open_export_output", { task_id: taskId }),
  });
}

export interface ExportTaskState {
  progress: ExportProgressPayload | null;
  complete: ExportCompletePayload | null;
  error: ExportErrorPayload | null;
}

/**
 * Subscribe to the `export:*` stream for one task. Events for other task ids
 * are ignored so two exports can't cross progress bars.
 */
export function useExportProgress(taskId: string | null): ExportTaskState {
  const [progress, setProgress] = useState<ExportProgressPayload | null>(null);
  const [complete, setComplete] = useState<ExportCompletePayload | null>(null);
  const [error, setError] = useState<ExportErrorPayload | null>(null);

  const onProgress = useCallback(
    (p: ExportProgressPayload) => {
      if (taskId && p.taskId === taskId) setProgress(p);
    },
    [taskId],
  );
  const onComplete = useCallback(
    (p: ExportCompletePayload) => {
      if (taskId && p.taskId === taskId) setComplete(p);
    },
    [taskId],
  );
  const onError = useCallback(
    (p: ExportErrorPayload) => {
      if (taskId && p.taskId === taskId) setError(p);
    },
    [taskId],
  );

  useEvent<ExportProgressPayload>("export:progress", onProgress);
  useEvent<ExportCompletePayload>("export:complete", onComplete);
  useEvent<ExportErrorPayload>("export:error", onError);

  return { progress, complete, error };
}
