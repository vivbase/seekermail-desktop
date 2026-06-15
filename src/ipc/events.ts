// Backend push events → query invalidation seam (07 §6, T024). Long-running ops
// return immediately and stream progress via events; components subscribe through
// `useEvent` rather than calling `listen` directly. `@tauri-apps/api` is imported
// here (inside `src/ipc/`) — allowed by the boundary rule.
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { QueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import type {
  AttachmentProgressPayload,
  AttachmentReadyPayload,
  AutoLoopDetectedPayload,
  AutoSentPayload,
  DraftDiscardedPayload,
  DraftReadyPayload,
  DraftUpdatedPayload,
  GteErrorPayload,
  GteFinishedPayload,
  GteProgressPayload,
  MailSummary,
  MailUpdatedPayload,
  PipelineErrorPayload,
  SyncCompletePayload,
  SyncErrorPayload,
  SyncProgressPayload,
  SyncStartedPayload,
} from "@shared/bindings";

import i18n from "@/i18n";
import { showToast } from "@/components/ui/Toast";

import { ipc, isTauri } from "./client";
// Local DTO mirror (T071) until `RiskEvent` lands in the generated bindings.
import type { RiskEvent } from "./legal";
// Agent-IM push notifications (T101). Best-effort; never carries mail content.
import * as notifications from "./notifications";

/** Known backend event names (02 §4). */
export type IpcEventName =
  | "sync:started"
  | "sync:progress"
  | "sync:complete"
  | "sync:error"
  | "mail:new"
  | "mail:updated"
  | "attachment:progress"
  | "attachment:ready"
  | "gte:progress"
  | "gte:finished"
  | "gte:error"
  | "risk:alert"
  | "query:new"
  | "query:expired"
  | "draft:ready"
  | "draft:updated"
  | "draft:discarded"
  | "auto:sent"
  | "auto:loop_detected"
  | "pipeline:error"
  | "export:progress"
  | "export:complete"
  | "export:error"
  | "wipe:progress"
  | "wipe:complete";

/**
 * Register the global event → invalidation table once at app start (main.tsx).
 * Returns a disposer that unlistens everything (used on HMR reload).
 */
export function registerIpcEvents(qc: QueryClient): () => void {
  if (!isTauri()) return () => {};
  const pending: Promise<UnlistenFn>[] = [];

  const on = <T>(name: IpcEventName, handler: (payload: T) => void) => {
    pending.push(listen<T>(name, (e) => handler(e.payload)));
  };

  on<SyncStartedPayload>("sync:started", (p) => {
    void qc.invalidateQueries({ queryKey: ["sync_state", p.accountId] });
  });
  on<SyncProgressPayload>("sync:progress", (p) => {
    // Optimistic: write progress directly, no refetch.
    qc.setQueryData(["sync_progress", p.accountId], p);
  });
  on<SyncCompletePayload>("sync:complete", (p) => {
    void qc.invalidateQueries({ queryKey: ["accounts"] });
    // The L0 list query keys are ["threads", accountId|"all", …] and
    // ["mails", accountId|"all", …]; all-accounts views use "all", so a
    // per-account key misses them. Prefix keys refresh every variant so a
    // completed sync surfaces fetched mail without a manual view switch.
    void qc.invalidateQueries({ queryKey: ["threads"] });
    void qc.invalidateQueries({ queryKey: ["mails"] });
    void qc.invalidateQueries({ queryKey: ["sync_state", p.accountId] });
    void qc.invalidateQueries({ queryKey: ["backfill", p.accountId] });
  });
  on<SyncErrorPayload>("sync:error", (p) => {
    void qc.invalidateQueries({ queryKey: ["sync_state", p.accountId] });
    qc.setQueryData(["sync_error", p.accountId], p);
  });
  on<MailSummary>("mail:new", (p) => {
    void qc.invalidateQueries({ queryKey: ["threads"] });
    void qc.invalidateQueries({ queryKey: ["mails"] });
    void qc.invalidateQueries({ queryKey: ["unread_count", p.accountId] });
  });
  on<MailUpdatedPayload>("mail:updated", (p) => {
    // Precise cache patch — no list refetch.
    qc.setQueryData(["mail", p.id], (prev: unknown) =>
      prev && typeof prev === "object" ? { ...prev, ...stripNulls(p) } : prev,
    );
  });
  on<AttachmentProgressPayload>("attachment:progress", (p) => {
    qc.setQueryData(["attachment_progress", p.attachmentId], p);
  });
  on<AttachmentReadyPayload>("attachment:ready", (p) => {
    void qc.invalidateQueries({
      queryKey: ["attachment_path", p.attachmentId],
    });
    void qc.invalidateQueries({ queryKey: ["attachments"] });
  });
  on<GteProgressPayload>("gte:progress", (p) => {
    // Optimistic: stash the latest GTE status for any status chip — no refetch.
    qc.setQueryData(["gte_status"], p);
  });
  on<GteFinishedPayload>("gte:finished", (p) => {
    qc.setQueryData(["gte_status"], {
      indexed: p.totalIndexed,
      totalPending: 0,
      ratePerSec: 0,
    });
    // New vectors → semantic results may have changed.
    void qc.invalidateQueries({ queryKey: ["search", "semantic"] });
  });
  on<GteErrorPayload>("gte:error", (p) => {
    qc.setQueryData(["gte_error"], p);
  });
  // New risk event (especially T4) → every riskEvents query refetches so the
  // non-dismissable banner appears without a page refresh (T071 §3.2, 02 §4).
  // T101: also refresh the pending-query surfaces and push a (content-free) OS
  // notification.
  on<RiskEvent>("risk:alert", () => {
    void qc.invalidateQueries({ queryKey: ["riskEvents"] });
    void qc.invalidateQueries({ queryKey: ["pendingQueries"] });
    void notifications.notifyRiskAlert();
  });
  // Agent-IM proactive queries (T101). The backend emit lands with v0.6
  // (T095/T097); these listeners are wired now so the sidebar badge, Pending
  // list, and channel refresh the moment those events start firing.
  on<{ queryId: string; accountId: string; priority: string }>("query:new", (p) => {
    void qc.invalidateQueries({ queryKey: ["pendingQueries"] });
    void qc.invalidateQueries({ queryKey: ["imMessages"] });
    if (p.priority === "high") void notifications.notifyQueryNew(p);
  });
  on<{ queryId: string; accountId: string; triggerType: string }>("query:expired", () => {
    void qc.invalidateQueries({ queryKey: ["pendingQueries"] });
    void qc.invalidateQueries({ queryKey: ["imMessages"] });
  });
  // Module E draft lifecycle (T078/T081, 02 §4): any draft change refreshes the
  // Pending page list + counts so cards appear/disappear without a refresh.
  const invalidatePendingDrafts = () => {
    void qc.invalidateQueries({ queryKey: ["pending_drafts"] });
    void qc.invalidateQueries({ queryKey: ["pending_counts"] });
  };
  on<DraftReadyPayload>("draft:ready", invalidatePendingDrafts);
  on<DraftUpdatedPayload>("draft:updated", (p) => {
    void qc.invalidateQueries({ queryKey: ["ai_draft", p.draftId] });
    invalidatePendingDrafts();
  });
  on<DraftDiscardedPayload>("draft:discarded", (p) => {
    void qc.invalidateQueries({ queryKey: ["ai_draft", p.draftId] });
    invalidatePendingDrafts();
  });
  // E3 full-auto pipeline (T085 backend / T086 UI). Wired defensively: each
  // handler is a safe no-op until the backend starts emitting these events.
  on<AutoSentPayload>("auto:sent", (p) => {
    invalidatePendingDrafts();
    void qc.invalidateQueries({ queryKey: ["audit"] });
    notifyAutoSent(qc, p);
  });
  on<AutoLoopDetectedPayload>("auto:loop_detected", () => {
    // Loop guard paused the thread's auto-replies; refresh queue surfaces.
    invalidatePendingDrafts();
  });
  on<PipelineErrorPayload>("pipeline:error", () => {
    // Generation failed for one mail; the queue list may have changed.
    invalidatePendingDrafts();
  });

  return () => {
    pending.forEach((p) => void p.then((un) => un()));
  };
}

/** Alias matching the T024 spec name. */
export const registerAllEvents = registerIpcEvents;

/** Undo window after an E3 auto-send — matches the backend `send_after` delay. */
const AUTO_SENT_UNDO_MS = 30_000;

/**
 * "Auto-sent" toast with a 30 s Undo (T086, F_E3 §5). The body is subject-free
 * (AI_MODES_DESIGN §4.2 — counts only, never mail content). Undo races the
 * delayed send via `cancel_send`; a draft that already left shows "Already
 * sent" instead of an error.
 */
function notifyAutoSent(qc: QueryClient, payload: AutoSentPayload): void {
  const t = i18n.getFixedT(null, "aiDrafts");
  showToast(t("toast_auto_sent"), {
    actionLabel: t("toast_auto_sent_undo"),
    durationMs: AUTO_SENT_UNDO_MS,
    onAction: () => {
      void (async () => {
        try {
          const result = await ipc("cancel_send", { pending_id: payload.draftId });
          if (result.cancelled) {
            showToast(t("toast_auto_send_cancelled"));
            void qc.invalidateQueries({ queryKey: ["pending_drafts"] });
            void qc.invalidateQueries({ queryKey: ["pending_counts"] });
          } else {
            showToast(t("toast_auto_send_already_sent"));
          }
        } catch {
          // The send already completed (or the queue entry is gone).
          showToast(t("toast_auto_send_already_sent"));
        }
      })();
    },
  });
}

/** Drop null fields so a partial update only overwrites provided keys. */
function stripNulls<T extends object>(obj: T): Partial<T> {
  const out: Partial<T> = {};
  for (const [k, v] of Object.entries(obj)) {
    if (v !== null && v !== undefined) (out as Record<string, unknown>)[k] = v;
  }
  return out;
}

/** Subscribe a component to a backend event for its lifetime. No-op off-Tauri. */
export function useEvent<T = unknown>(name: IpcEventName, handler: (payload: T) => void): void {
  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: UnlistenFn | undefined;
    void listen<T>(name, (event) => handler(event.payload)).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, [name, handler]);
}
