// Global transient toast (T078/T081). The repo previously had only the
// component-scoped UndoToast; flows that navigate between routes (E1 AI reply,
// E6 draft review) need a toast that survives the route change, so the queue
// lives in a tiny zustand store and the viewport mounts once in AppShell.
// Optional action button covers the 5-second undo affordance (F_E6 §3.3).
import { useEffect } from "react";
import { create } from "zustand";

import { cn } from "@/lib/cn";

/** Default auto-dismiss delay; undo toasts pass 5000 explicitly (F_E6 §3.3). */
const DEFAULT_DURATION_MS = 2800;

export interface ToastOptions {
  /** Label of the optional inline action button (e.g. "Undo"). */
  actionLabel?: string;
  /** Invoked when the action button is clicked; the toast dismisses after. */
  onAction?: () => void;
  durationMs?: number;
}

interface ToastItem extends Required<Pick<ToastOptions, "durationMs">> {
  id: number;
  message: string;
  actionLabel?: string;
  onAction?: () => void;
}

interface ToastStore {
  toasts: ToastItem[];
  push: (message: string, options?: ToastOptions) => number;
  dismiss: (id: number) => void;
}

let nextToastId = 0;

export const useToastStore = create<ToastStore>((set) => ({
  toasts: [],
  push: (message, options) => {
    nextToastId += 1;
    const item: ToastItem = {
      id: nextToastId,
      message,
      actionLabel: options?.actionLabel,
      onAction: options?.onAction,
      durationMs: options?.durationMs ?? DEFAULT_DURATION_MS,
    };
    set((s) => ({ toasts: [...s.toasts, item] }));
    return item.id;
  },
  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
}));

/** Imperative entry point usable outside React components (e.g. query hooks). */
export function showToast(message: string, options?: ToastOptions): number {
  return useToastStore.getState().push(message, options);
}

// ── Single toast ──────────────────────────────────────────────────────────────

function ToastCard({ toast }: { toast: ToastItem }) {
  const dismiss = useToastStore((s) => s.dismiss);

  useEffect(() => {
    const timer = setTimeout(() => dismiss(toast.id), toast.durationMs);
    return () => clearTimeout(timer);
  }, [toast.id, toast.durationMs, dismiss]);

  return (
    <div
      role="status"
      aria-live="polite"
      aria-atomic="true"
      className={cn(
        "pointer-events-auto flex w-80 items-center justify-between gap-3",
        "rounded-card bg-p9 px-4 py-3 shadow-card",
      )}
    >
      <p className="font-ui text-sm text-surface">{toast.message}</p>
      {toast.actionLabel && (
        <button
          type="button"
          onClick={() => {
            toast.onAction?.();
            dismiss(toast.id);
          }}
          className={cn(
            "shrink-0 rounded-chip border border-p7 px-3 py-1 font-ui text-xs text-surface",
            "hover:bg-p8 focus:outline-none focus-visible:ring-2 focus-visible:ring-surface",
          )}
        >
          {toast.actionLabel}
        </button>
      )}
    </div>
  );
}

// ── Viewport (mounted once in AppShell) ───────────────────────────────────────

export function ToastViewport() {
  const toasts = useToastStore((s) => s.toasts);
  if (toasts.length === 0) return null;
  return (
    <div className="pointer-events-none fixed bottom-6 end-6 z-50 flex flex-col items-end gap-2">
      {toasts.map((toast) => (
        <ToastCard key={toast.id} toast={toast} />
      ))}
    </div>
  );
}
