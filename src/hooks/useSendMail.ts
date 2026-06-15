// Send-mail controller hook (T044). Wraps ipc send_mail + cancel_send with a
// 10-second undo window. Components consume this; no direct ipc imports outside
// src/ipc/ (07 §6). The hook is self-contained: it manages the pending-id state
// and countdown timer internally.

import { useCallback, useEffect, useRef, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import type { CancelSendResult, SendMailParams, SendMailResult } from "@shared/bindings";
import { ipc } from "@/ipc/client";

// ── Types ────────────────────────────────────────────────────────────────────

/** Duration (ms) for which the undo UI is visible. */
const UNDO_UI_DURATION_MS = 10_000;
/** Polling interval (ms) for the countdown display. */
const COUNTDOWN_TICK_MS = 100;

export type SendStatus =
  | { phase: "idle" }
  | { phase: "sending" }
  | { phase: "undo"; pendingId: string; remainingMs: number }
  | { phase: "sent" }
  | { phase: "cancelled" }
  | { phase: "error"; message: string };

export interface UseSendMailReturn {
  status: SendStatus;
  /** Trigger send. Returns the result or throws on validation/ipc failure. */
  send: (params: SendMailParams) => Promise<SendMailResult>;
  /** Cancel the pending send within the undo window. */
  cancel: () => void;
  /** Reset to idle (e.g. after navigating away). */
  resetStatus: () => void;
}

// ── Hook ─────────────────────────────────────────────────────────────────────

export function useSendMail(): UseSendMailReturn {
  const [status, setStatus] = useState<SendStatus>({ phase: "idle" });

  // Timer refs — kept in refs to survive re-renders without triggering effects.
  const countdownIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const undoDeadlineRef = useRef<number | null>(null);
  const isMountedRef = useRef(true);

  useEffect(() => {
    isMountedRef.current = true;
    return () => {
      isMountedRef.current = false;
      if (countdownIntervalRef.current !== null) {
        clearInterval(countdownIntervalRef.current);
      }
    };
  }, []);

  function clearCountdown() {
    if (countdownIntervalRef.current !== null) {
      clearInterval(countdownIntervalRef.current);
      countdownIntervalRef.current = null;
    }
    undoDeadlineRef.current = null;
  }

  function startUndoCountdown(pendingId: string) {
    const deadline = Date.now() + UNDO_UI_DURATION_MS;
    undoDeadlineRef.current = deadline;

    // Set initial state immediately.
    if (isMountedRef.current) {
      setStatus({ phase: "undo", pendingId, remainingMs: UNDO_UI_DURATION_MS });
    }

    countdownIntervalRef.current = setInterval(() => {
      if (!isMountedRef.current) {
        clearCountdown();
        return;
      }
      const remaining = (undoDeadlineRef.current ?? 0) - Date.now();
      if (remaining <= 0) {
        clearCountdown();
        if (isMountedRef.current) {
          setStatus({ phase: "sent" });
        }
      } else {
        setStatus({ phase: "undo", pendingId, remainingMs: remaining });
      }
    }, COUNTDOWN_TICK_MS);
  }

  // ── send_mail mutation ───────────────────────────────────────────────────

  const sendMutation = useMutation<SendMailResult, Error, SendMailParams>({
    mutationFn: (params) => ipc("send_mail", { params }),
    onMutate: () => {
      if (isMountedRef.current) setStatus({ phase: "sending" });
    },
    onSuccess: (result) => {
      startUndoCountdown(result.pendingId);
    },
    onError: (err) => {
      if (isMountedRef.current) {
        setStatus({ phase: "error", message: err.message });
      }
    },
  });

  // ── cancel_send mutation ─────────────────────────────────────────────────

  const cancelMutation = useMutation<CancelSendResult, Error, string>({
    mutationFn: (pendingId) => ipc("cancel_send", { pending_id: pendingId }),
    onSuccess: (result) => {
      clearCountdown();
      if (isMountedRef.current) {
        setStatus(result.cancelled ? { phase: "cancelled" } : { phase: "sent" });
      }
    },
    onError: () => {
      clearCountdown();
      // If cancel fails we treat the message as sent (most conservative).
      if (isMountedRef.current) setStatus({ phase: "sent" });
    },
  });

  // ── Public API ───────────────────────────────────────────────────────────

  const send = useCallback(
    (params: SendMailParams): Promise<SendMailResult> => {
      clearCountdown();
      return sendMutation.mutateAsync(params);
    },
    [sendMutation],
  );

  const cancel = useCallback(() => {
    if (status.phase !== "undo") return;
    const { pendingId } = status;
    clearCountdown();
    cancelMutation.mutate(pendingId);
  }, [status, cancelMutation]);

  const resetStatus = useCallback(() => {
    clearCountdown();
    if (isMountedRef.current) setStatus({ phase: "idle" });
  }, []);

  return { status, send, cancel, resetStatus };
}
