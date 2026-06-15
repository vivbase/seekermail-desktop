// Undo toast for archive / delete actions (T038, dev/07 §9).
// Non-optimistic destructive operations show this 6-second toast.
// Self-contained: accepts message + onUndo callback + auto-dismiss timer.
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/cn";

const DURATION_MS = 6000;

export interface UndoToastProps {
  /** Label text shown in the toast (e.g. "Archived" or "Deleted"). */
  message: string;
  /** Called when the user clicks Undo before the timer expires. */
  onUndo: () => void;
  /** Called when the timer expires with no Undo click. */
  onExpire?: () => void;
  /** Called when the toast is dismissed (either by Undo or expiry). */
  onDismiss?: () => void;
}

export function UndoToast({ message, onUndo, onExpire, onDismiss }: UndoToastProps) {
  const { t } = useTranslation("list");
  const [remaining, setRemaining] = useState(DURATION_MS);
  const [visible, setVisible] = useState(true);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const startRef = useRef(Date.now());

  useEffect(() => {
    startRef.current = Date.now();

    intervalRef.current = setInterval(() => {
      const elapsed = Date.now() - startRef.current;
      const left = Math.max(0, DURATION_MS - elapsed);
      setRemaining(left);

      if (left === 0) {
        if (intervalRef.current) clearInterval(intervalRef.current);
        setVisible(false);
        onExpire?.();
        onDismiss?.();
      }
    }, 100);

    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleUndo = () => {
    if (intervalRef.current) clearInterval(intervalRef.current);
    setVisible(false);
    onUndo();
    onDismiss?.();
  };

  if (!visible) return null;

  const secondsLeft = Math.ceil(remaining / 1000);
  const progressPct = (remaining / DURATION_MS) * 100;

  return (
    <div
      role="status"
      aria-live="polite"
      aria-atomic="true"
      className={cn(
        "pointer-events-auto flex w-80 flex-col overflow-hidden rounded-card bg-p9 shadow-card",
      )}
    >
      <div className="flex items-center justify-between gap-3 px-4 py-3">
        <p className="font-ui text-sm text-surface">{message}</p>

        <button
          type="button"
          onClick={handleUndo}
          aria-label={`${t("undo")} — ${t("undo")} available for ${secondsLeft} seconds`}
          className="shrink-0 rounded-chip border border-p7 px-3 py-1 font-ui text-xs text-surface hover:bg-p8 focus:outline-none focus-visible:ring-2 focus-visible:ring-surface"
        >
          {t("undo")}
        </button>
      </div>

      {/* Progress bar */}
      <div className="h-0.5 w-full bg-p8">
        <div
          aria-hidden="true"
          className="h-full bg-sage transition-[width] duration-100"
          style={{ width: `${progressPct}%` }}
        />
      </div>
    </div>
  );
}

// ── Imperative hook for spawning a single toast at a time ─────────────────────

interface ToastState {
  id: number;
  message: string;
  onUndo: () => void;
}

let _toastId = 0;

/**
 * Tiny imperative hook that manages a single UndoToast instance.
 * Returns { toastEl, showUndoToast }.
 *
 * Usage:
 *   const { toastEl, showUndoToast } = useUndoToast();
 *   // Later: showUndoToast("Archived", handleUndo);
 *   // Render: {toastEl} somewhere in the tree (e.g. fixed bottom-right).
 */
export function useUndoToast() {
  const [toast, setToast] = useState<ToastState | null>(null);

  const showUndoToast = (message: string, onUndo: () => void) => {
    _toastId += 1;
    setToast({ id: _toastId, message, onUndo });
  };

  const toastEl = toast ? (
    <UndoToast
      key={toast.id}
      message={toast.message}
      onUndo={toast.onUndo}
      onDismiss={() => setToast(null)}
    />
  ) : null;

  return { toastEl, showUndoToast };
}
