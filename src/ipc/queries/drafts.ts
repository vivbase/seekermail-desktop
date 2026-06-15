// TanStack Query hooks for compose drafts (T045) and Module E AI drafts
// (T078 E1 manual reply, T081 E6 inline review). Components consume these,
// never `ipc()` directly (07 §6).
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import type {
  AiDraft,
  ApproveDraftResult,
  IpcError,
  MailDetail,
  SaveDraftParams,
} from "@shared/bindings";

import { ipc } from "../client";
import { showToast } from "@/components/ui/Toast";
import { markdownToPlainText } from "@/lib/markdown";

export const draftKeys = {
  detail: (id: string) => ["draft", id] as const,
};

export const aiDraftKeys = {
  /** Prefix key — events.ts invalidates on draft:* pushes. */
  pendingRoot: ["pending_drafts"] as const,
  pending: (accountId?: string | null) => ["pending_drafts", accountId ?? "all"] as const,
  detail: (id: string) => ["ai_draft", id] as const,
};

/** Load a draft by id (used when resuming a compose). */
export function useDraft(id: string | null | undefined) {
  return useQuery({
    queryKey: draftKeys.detail(id ?? ""),
    queryFn: () => ipc("get_draft", { id: id as string }),
    enabled: !!id,
    staleTime: 0,
  });
}

/** Upsert (autosave) a draft. Returns the persisted row, including its id. */
export function useSaveDraft() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (params: SaveDraftParams) => ipc("save_draft", { params }),
    onSuccess: (draft) => qc.setQueryData(draftKeys.detail(draft.id), draft),
  });
}

/** Delete a draft (on send or discard). */
export function useDeleteDraft() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => ipc("delete_draft", { id }),
    onSuccess: (_d, id) => void qc.removeQueries({ queryKey: draftKeys.detail(id) }),
  });
}

// ── Module E — AI drafts ──────────────────────────────────────────────────────

/**
 * Compose seed carried through router state when a flow hands an AI draft to
 * the /compose route (E1 success path, E6 "Open in Compose"). The compose
 * route applies it via the compose store on mount.
 */
export interface AiComposeSeed {
  accountId: string;
  to: string;
  subject: string;
  body: string;
  inReplyTo: string | null;
  aiDraftId: string;
}

/** Build the /compose router-state seed from a generated AI draft. */
export function buildAiComposeSeed(draft: AiDraft): AiComposeSeed {
  return {
    accountId: draft.accountId,
    to: draft.toAddr.email,
    subject: draft.subject,
    body: markdownToPlainText(draft.bodyCurrent),
    inReplyTo: draft.triggerMailId,
    aiDraftId: draft.id,
  };
}

/** E6 inline review list — drafts awaiting human review on /pending (02 §Module E). */
export function usePendingDrafts(accountId?: string | null) {
  return useQuery({
    queryKey: aiDraftKeys.pending(accountId),
    queryFn: () =>
      ipc("list_pending_drafts", { params: { accountId: accountId ?? null, limit: null } }),
    staleTime: 30_000,
  });
}

/** Counts surfaced by the Pending nav badge + filter chips (T083). */
export interface PendingCounts {
  draftCount: number;
  decisionCount: number;
}

/**
 * Derived Pending counts (T083) — no extra IPC command: reuses the cached
 * `list_pending_drafts` result. `decisionCount` joins in when the T095/T096
 * `list_pending_queries` surface lands; until then it reads zero. The
 * `draft:*` events invalidate `pending_drafts`, which re-renders consumers.
 */
export function usePendingCounts(): PendingCounts {
  const drafts = usePendingDrafts();
  return {
    draftCount: drafts.data?.length ?? 0,
    // T1–T6 decision queries ship with T095/T096 (`list_pending_queries`).
    decisionCount: 0,
  };
}

/**
 * O(1) lookup set of mail ids that have a pending AI draft (T083 L0 badge).
 * ThreadCard checks membership per row; the set is memoised on the cached
 * pending-drafts list so the virtualized stream stays O(n).
 */
export function useAiDraftTriggerMailIds(): ReadonlySet<string> {
  const { data } = usePendingDrafts();
  return useMemo(() => new Set((data ?? []).map((d) => d.triggerMailId)), [data]);
}

export interface RequestAiReplyVars {
  /** The mail being replied to — also seeds the blank-reply fallback. */
  mail: MailDetail;
  instruction?: string;
}

/**
 * E1 manual AI reply (T078, F_E1 §4.4). On success, navigates to /compose with
 * the draft pre-filled; on failure, shows a toast and opens a blank reply so
 * the user is never blocked. An unconfigured provider routes to AI settings.
 */
export function useRequestAiReply() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const { t } = useTranslation("aiDrafts");

  return useMutation<AiDraft, IpcError, RequestAiReplyVars>({
    mutationFn: ({ mail, instruction }) =>
      ipc("request_ai_reply", { params: { mailId: mail.id, instruction: instruction ?? null } }),
    onSuccess: (draft, { mail }) => {
      void qc.invalidateQueries({ queryKey: aiDraftKeys.pendingRoot });
      const aiSeed = buildAiComposeSeed(draft);
      // Threading follows the local mail id, same as MailToolbar's reply path.
      aiSeed.inReplyTo = mail.id;
      void navigate("/compose", { state: { mode: "reply", aiSeed } });
    },
    onError: (err, { mail }) => {
      const notConfigured =
        err.code === "AI_PROVIDER_UNREACHABLE" && (err.detail ?? "").includes("not_configured");
      if (notConfigured) {
        // No provider for this account → guide the user to Module F settings.
        showToast(t("toast_ai_provider_not_configured"));
        void navigate("/settings/ai");
        return;
      }
      // Any other failure: toast + blank reply compose (F_E1 §4.4 — never block).
      showToast(t("toast_ai_draft_failed"));
      void navigate("/compose", { state: { mode: "reply", mail } });
    },
  });
}

export interface RegenerateDraftVars {
  id: string;
  instruction?: string;
}

/** Re-run generation for an AI draft (E1 compose toolbar + E6 panel). */
export function useRegenerateDraft() {
  const qc = useQueryClient();
  return useMutation<AiDraft, IpcError, RegenerateDraftVars>({
    mutationFn: ({ id, instruction }) =>
      ipc("regenerate_draft", { params: { id, instruction: instruction ?? null } }),
    onSuccess: (draft) => {
      qc.setQueryData(aiDraftKeys.detail(draft.id), draft);
      void qc.invalidateQueries({ queryKey: aiDraftKeys.pendingRoot });
    },
  });
}

/** E6 "Send Now" — sends the draft over SMTP and resolves its queue entry. */
export function useApproveDraft() {
  const qc = useQueryClient();
  return useMutation<ApproveDraftResult, IpcError, string>({
    mutationFn: (id) => ipc("approve_draft", { id }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: aiDraftKeys.pendingRoot });
      void qc.invalidateQueries({ queryKey: ["pending_counts"] });
    },
  });
}

export interface DiscardDraftVars {
  id: string;
  reason?: string;
}

/** E6 "Discard Draft". */
export function useDiscardDraft() {
  const qc = useQueryClient();
  return useMutation<null, IpcError, DiscardDraftVars>({
    mutationFn: ({ id, reason }) => ipc("discard_draft", { id, reason: reason ?? null }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: aiDraftKeys.pendingRoot });
      void qc.invalidateQueries({ queryKey: ["pending_counts"] });
    },
  });
}

export interface UpdateDraftBodyVars {
  id: string;
  bodyCurrent: string;
}

/** Persist the user's inline edits to an AI draft body (sets `is_edited`). */
export function useUpdateDraftBody() {
  const qc = useQueryClient();
  const { t } = useTranslation("aiDrafts");
  return useMutation<AiDraft, IpcError, UpdateDraftBodyVars>({
    mutationFn: ({ id, bodyCurrent }) =>
      ipc("update_draft_body", { id, body_current: bodyCurrent }),
    onSuccess: (draft) => {
      qc.setQueryData(aiDraftKeys.detail(draft.id), draft);
      void qc.invalidateQueries({ queryKey: aiDraftKeys.pendingRoot });
    },
    // Autosave failure must be visible — silent loss of edits is worse than
    // a noisy toast (T090 §3).
    onError: () => {
      showToast(t("draft_autosave_failed"));
    },
  });
}

/**
 * T090 backstop for the frontend-driven undo window: rolls a draft back to
 * `pending` if an approve was dispatched anyway (e.g. window close raced the
 * undo). The common cancel path never calls this — it just clears the timer.
 */
export function useCancelDraftSend() {
  const qc = useQueryClient();
  return useMutation<AiDraft, IpcError, string>({
    mutationFn: (id) => ipc("cancel_draft_send", { id }),
    onSuccess: (draft) => {
      qc.setQueryData(aiDraftKeys.detail(draft.id), draft);
      void qc.invalidateQueries({ queryKey: aiDraftKeys.pendingRoot });
      void qc.invalidateQueries({ queryKey: ["pending_counts"] });
    },
  });
}
