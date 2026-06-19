// Pending page (T081, F_E6 §3, root CLAUDE.md "Pending Page — Two Card Types").
// The single unified inbox for everything needing human attention:
//   data-type="decision" — T1–T6 proactive queries (cards land with T095/T096)
//   data-type="draft"    — E2 semi-auto AI drafts reviewed inline (this card)
// Filter chips + green draft banner + merged card list + the inline DraftPanel.
// There is deliberately NO standalone draft-review route or nav item.
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AiDraft } from "@shared/bindings";

import {
  useApproveDraft,
  useDiscardDraft,
  usePendingDrafts,
  useRegenerateDraft,
} from "@/ipc/queries/drafts";
import { useAccounts } from "@/ipc/queries/accounts";
import { usePendingQueries } from "@/ipc/queries/queries";
import { useUi, type PendingFilter } from "@/stores/ui";
import { showToast } from "@/components/ui/Toast";
import PageBack from "@/components/layout/PageBack";
import { DraftCard } from "@/components/pending/DraftCard";
import { DecisionCard } from "@/components/pending/DecisionCard";
import { DraftPanel, DRAFT_EDITOR_ID } from "@/components/pending/DraftPanel";
import type { PendingQuery } from "@/ipc/pendingQueries";
import { cn } from "@/lib/cn";

/**
 * Undo window for Send / Discard (F_E6 §3.3). Both actions use the
 * frontend-delayed-invoke pattern from dev/02 §approve_draft (the same
 * strategy T090 documents): the mutation only fires after the undo toast
 * expires, so "Undo" simply cancels the scheduled call — no backend
 * un-discard / cancel_send round-trip is needed in the common path.
 */
const UNDO_WINDOW_MS = 5_000;

/** Error codes that surface the credential notice instead of a retry toast. */
const CREDENTIAL_ERROR_CODES = new Set(["AUTH_INVALID_CREDENTIALS", "SMTP_SEND_FAILED"]);

// ── Delayed (undoable) actions ────────────────────────────────────────────────

interface ScheduledAction {
  timer: ReturnType<typeof setTimeout>;
  fire: () => void;
}

/**
 * Schedule cancellable deferred mutations. Anything still pending on unmount
 * fires immediately so a route change can't silently drop a send/discard.
 */
function useDelayedActions() {
  const actionsRef = useRef(new Map<string, ScheduledAction>());

  useEffect(() => {
    const actions = actionsRef.current;
    return () => {
      for (const { timer, fire } of actions.values()) {
        clearTimeout(timer);
        fire();
      }
      actions.clear();
    };
  }, []);

  const schedule = useCallback((key: string, fire: () => void, delayMs: number) => {
    const timer = setTimeout(() => {
      actionsRef.current.delete(key);
      fire();
    }, delayMs);
    actionsRef.current.set(key, { timer, fire });
  }, []);

  const cancel = useCallback((key: string): boolean => {
    const entry = actionsRef.current.get(key);
    if (!entry) return false;
    clearTimeout(entry.timer);
    actionsRef.current.delete(key);
    return true;
  }, []);

  return { schedule, cancel };
}

// ── Merged card list ──────────────────────────────────────────────────────────

/**
 * Unified Pending item. Decision cards (T1–T6 queries, T095/T096) and E2 review
 * drafts coexist here per root CLAUDE.md "Pending Page — Two Card Types"; the
 * filter + rendering plumbing branches on `type`.
 */
type PendingItem = { type: "decision"; query: PendingQuery } | { type: "draft"; draft: AiDraft };

// ── Route ─────────────────────────────────────────────────────────────────────

export default function Pending() {
  const { t } = useTranslation(["aiDrafts", "nav", "common"]);

  const pendingFilter = useUi((s) => s.pendingFilter);
  const setPendingFilter = useUi((s) => s.setPendingFilter);

  const { data: drafts = [] } = usePendingDrafts();
  const { data: queries = [] } = usePendingQueries();
  const { data: accounts = [] } = useAccounts();

  const approve = useApproveDraft();
  const discard = useDiscardDraft();
  const regenerate = useRegenerateDraft();
  const { schedule, cancel } = useDelayedActions();

  // Cards optimistically hidden while their undo toast runs.
  const [hiddenIds, setHiddenIds] = useState<ReadonlySet<string>>(new Set());
  // Accounts whose approve failed with a credential error (inline notice).
  const [credErrorIds, setCredErrorIds] = useState<ReadonlySet<string>>(new Set());
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const visibleDrafts = drafts.filter((d) => !hiddenIds.has(d.id));
  const selectedDraft = visibleDrafts.find((d) => d.id === selectedId) ?? null;

  // Decision cards (T1–T6 queries) sort ahead of review drafts (higher urgency).
  const decisionCount = queries.length;
  const draftCount = visibleDrafts.length;
  const items: PendingItem[] = [
    ...queries.map((query): PendingItem => ({ type: "decision", query })),
    ...visibleDrafts.map((draft): PendingItem => ({ type: "draft", draft })),
  ];
  const filteredItems = items.filter(
    (item) => pendingFilter === "all" || item.type === pendingFilter,
  );

  // The detail container is shared with the future DecisionPanel (F_E6 §3.3);
  // `panelMode` switches which panel renders into it.
  const panelMode: "decision" | "draft" | null = selectedDraft ? "draft" : null;

  const setHidden = useCallback((id: string, hidden: boolean) => {
    setHiddenIds((prev) => {
      const next = new Set(prev);
      if (hidden) next.add(id);
      else next.delete(id);
      return next;
    });
  }, []);

  // ── Send / discard with 5 s undo ──────────────────────────────────────────

  const handleSend = useCallback(
    (draft: AiDraft) => {
      setSelectedId(null);
      setHidden(draft.id, true);
      // T090 §3: the undo window is announced up front; "Cancel" clears the
      // scheduled invoke (no backend round-trip in the common path).
      showToast(t("aiDrafts:draft_send_pending_toast"), {
        actionLabel: t("aiDrafts:draft_cancel_send"),
        durationMs: UNDO_WINDOW_MS,
        onAction: () => {
          if (cancel(draft.id)) {
            setHidden(draft.id, false);
            showToast(t("aiDrafts:draft_send_cancelled"));
          }
        },
      });
      schedule(
        draft.id,
        () =>
          approve.mutate(draft.id, {
            onSuccess: () => setHidden(draft.id, false),
            onError: (err) => {
              setHidden(draft.id, false);
              // Double-dispatch race: the backend already sent this draft.
              // Not an error from the user's perspective (T090 §6).
              const conflict =
                (err.code as string) === "CONFLICT" || /already sent/i.test(err.message);
              if (conflict) {
                showToast(t("aiDrafts:draft_already_sent"));
                return;
              }
              if (CREDENTIAL_ERROR_CODES.has(err.code)) {
                setCredErrorIds((prev) => new Set(prev).add(draft.id));
              }
              showToast(t("aiDrafts:draft_send_failed"));
            },
          }),
        UNDO_WINDOW_MS,
      );
    },
    [approve, cancel, schedule, setHidden, t],
  );

  const handleDiscard = useCallback(
    (draft: AiDraft) => {
      setSelectedId(null);
      setHidden(draft.id, true);
      showToast(t("aiDrafts:toast_draft_discarded"), {
        actionLabel: t("aiDrafts:undo"),
        durationMs: UNDO_WINDOW_MS,
        onAction: () => {
          if (cancel(draft.id)) setHidden(draft.id, false);
        },
      });
      schedule(
        draft.id,
        () => discard.mutate({ id: draft.id }, { onSettled: () => setHidden(draft.id, false) }),
        UNDO_WINDOW_MS,
      );
    },
    [cancel, discard, schedule, setHidden, t],
  );

  const handleRegenerate = useCallback(
    (draft: AiDraft) => {
      regenerate.mutate({ id: draft.id });
    },
    [regenerate],
  );

  // ── Keyboard shortcuts (F_E6 §3.4 — active while the panel is open) ───────

  useEffect(() => {
    if (!selectedDraft) return;

    function onKeyDown(e: KeyboardEvent) {
      const target = e.target as HTMLElement | null;
      const inEditor =
        !!target &&
        (target.tagName === "TEXTAREA" ||
          target.tagName === "INPUT" ||
          target.tagName === "SELECT" ||
          target.isContentEditable);

      if (e.key === "Escape") {
        e.preventDefault();
        if (inEditor) target?.blur();
        setSelectedId(null);
        return;
      }
      // Letter shortcuts never fire while typing in a field.
      if (inEditor || e.metaKey || e.ctrlKey || e.altKey) return;
      if (!selectedDraft) return;
      const draft = selectedDraft;

      switch (e.key.toLowerCase()) {
        case "j":
        case "k": {
          e.preventDefault();
          const ids = visibleDrafts.map((d) => d.id);
          const at = ids.indexOf(draft.id);
          if (at < 0) return;
          const next = e.key.toLowerCase() === "j" ? at + 1 : at - 1;
          const nextId = ids[next];
          if (nextId !== undefined) setSelectedId(nextId);
          return;
        }
        case "s":
          e.preventDefault();
          handleSend(draft);
          return;
        case "e":
          e.preventDefault();
          document.getElementById(DRAFT_EDITOR_ID)?.focus();
          return;
        case "r":
          e.preventDefault();
          handleRegenerate(draft);
          return;
        case "d":
          e.preventDefault();
          handleDiscard(draft);
          return;
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [selectedDraft, visibleDrafts, handleSend, handleDiscard, handleRegenerate]);

  // ── Filter chips ──────────────────────────────────────────────────────────

  const chips: { key: PendingFilter; label: string; count: number }[] = [
    { key: "all", label: t("aiDrafts:pending_filter_all"), count: decisionCount + draftCount },
    {
      key: "decision",
      label: t("aiDrafts:pending_filter_needs_decision"),
      count: decisionCount,
    },
    { key: "draft", label: t("aiDrafts:pending_filter_draft_ready"), count: draftCount },
  ];

  return (
    <section className="flex h-full overflow-hidden">
      {/* List column */}
      <div className="min-w-0 flex-1 overflow-y-auto px-8 py-8">
        <div className="mx-auto w-full max-w-2xl">
          <PageBack to="/" labelKey="back_to_dashboard" />
          <p className="section-label mb-2">{t("nav:nav_section_overview")}</p>
          <h1 className="font-display text-4xl italic text-p10">{t("nav:nav_pending")}</h1>
          <p className="mt-2 font-body text-sm text-p8">{t("common:pending_desc")}</p>

          {/* Filter chips */}
          <div
            role="tablist"
            aria-label={t("nav:nav_pending")}
            className="mt-6 flex items-center gap-2"
          >
            {chips.map((chip) => {
              const active = pendingFilter === chip.key;
              return (
                <button
                  key={chip.key}
                  type="button"
                  role="tab"
                  aria-selected={active}
                  onClick={() => setPendingFilter(chip.key)}
                  className={cn(
                    "rounded-chip px-3 py-1.5 font-ui text-[10px] font-semibold uppercase tracking-wider transition-colors",
                    "focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
                    active ? "bg-p9 text-white" : "border border-divider text-p8 hover:bg-p4",
                  )}
                >
                  {chip.label}
                  <span className={cn("ms-1.5 font-mono", active ? "text-p5" : "text-p7")}>
                    {chip.count}
                  </span>
                </button>
              );
            })}
          </div>

          {/* Green draft banner (F_E6 §3.1) */}
          {draftCount > 0 && (
            <div className="border-green/30 bg-green/10 mt-4 flex items-center justify-between gap-3 rounded-card border px-4 py-2.5">
              <p className="font-body text-sm text-p9">
                {t("aiDrafts:draft_banner_count", { count: draftCount })}
              </p>
              <button
                type="button"
                onClick={() => setPendingFilter("draft")}
                className="hover:bg-green/15 shrink-0 rounded-chip px-2.5 py-1 font-ui text-[10px] font-semibold uppercase tracking-wider text-green focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
              >
                {t("aiDrafts:draft_banner_review_all")}
              </button>
            </div>
          )}

          {/* Merged card list */}
          <div className="mt-5 flex flex-col gap-3">
            {filteredItems.length === 0 ? (
              <div className="rounded-card border border-divider bg-surface p-6 shadow-card">
                <p className="font-body text-sm text-p7">{t("aiDrafts:pending_empty")}</p>
              </div>
            ) : (
              filteredItems.map((item) =>
                item.type === "decision" ? (
                  <DecisionCard key={item.query.id} query={item.query} />
                ) : (
                  <DraftCard
                    key={item.draft.id}
                    draft={item.draft}
                    account={accounts.find((a) => a.id === item.draft.accountId)}
                    selected={item.draft.id === selectedId}
                    onOpen={setSelectedId}
                  />
                ),
              )
            )}
          </div>
        </div>
      </div>

      {/* Shared detail container — DraftPanel now, DecisionPanel with T095. */}
      {panelMode === "draft" && selectedDraft && (
        <DraftPanel
          draft={selectedDraft}
          credentialError={credErrorIds.has(selectedDraft.id)}
          regenerating={regenerate.isPending}
          onSend={() => handleSend(selectedDraft)}
          onDiscard={() => handleDiscard(selectedDraft)}
          onRegenerate={() => handleRegenerate(selectedDraft)}
          onClose={() => setSelectedId(null)}
        />
      )}
    </section>
  );
}
