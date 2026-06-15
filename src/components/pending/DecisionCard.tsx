// Pending "Needs Decision" card (T099, F_I4 §2). Renders an I3/I4 QA card:
// trigger badge, an "Open original email" link, the question, quick-option chips,
// a free-text note, and Submit / Skip. State drives the logical left-border color
// (pending → interactive; T4 → red; error → amber). Submit calls answer_query,
// Skip (after a confirm) calls skip_query. Coexists with DraftCard on the Pending
// page per root CLAUDE.md "Pending Page — Two Card Types".
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

import { useAnswerQuery, useSkipQuery } from "@/ipc/queries/queries";
import {
  parseQaCard,
  SKIP_VALUE,
  VIEW_EMAIL_VALUE,
  type PendingQuery,
  type QaCardOption,
} from "@/ipc/pendingQueries";
import ConfirmDialog from "@/components/ui/ConfirmDialog";
import { cn } from "@/lib/cn";

interface DecisionCardProps {
  query: PendingQuery;
}

/** Left-border accent token by card state (F_I4 §2.2). */
function borderVar(status: string, isT4: boolean, errored: boolean): string {
  if (errored) return "var(--amber)";
  switch (status) {
    case "answered":
      return "var(--green)";
    case "skipped":
    case "expired":
      return "var(--p5)";
    default:
      return isT4 ? "var(--red)" : "var(--p9)";
  }
}

export function DecisionCard({ query }: DecisionCardProps) {
  const { t } = useTranslation("team");
  const navigate = useNavigate();
  const answer = useAnswerQuery();
  const skip = useSkipQuery();

  const card = parseQaCard(query.options);
  const isT4 = query.triggerType === "T4";
  const multi = card?.multiSelect ?? false;
  // Answerable chips exclude the Skip + View-email pseudo-options.
  const chipOptions: QaCardOption[] = (card?.options ?? []).filter(
    (o) => o.value !== SKIP_VALUE && o.value !== VIEW_EMAIL_VALUE,
  );
  const hasViewEmail = (card?.options ?? []).some((o) => o.value === VIEW_EMAIL_VALUE);

  const [selected, setSelected] = useState<string[]>([]);
  const [freeText, setFreeText] = useState("");
  const [skipOpen, setSkipOpen] = useState(false);
  const [errored, setErrored] = useState(false);

  const pending = query.status === "pending";
  const busy = answer.isPending || skip.isPending;
  const canSubmit = pending && (selected.length > 0 || freeText.trim().length > 0) && !busy;

  const toggle = (id: string) => {
    setSelected((prev) => {
      if (multi) {
        return prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id];
      }
      return prev.includes(id) ? [] : [id];
    });
  };

  const openEmail = () => {
    if (query.mailId) void navigate(`/mail/${query.mailId}`);
  };

  const submit = () => {
    if (!canSubmit) return;
    setErrored(false);
    const payload = JSON.stringify({
      selectedOptionIds: selected,
      freeText: freeText.trim() || null,
    });
    answer.mutate({ id: query.id, answer: payload }, { onError: () => setErrored(true) });
  };

  const confirmSkip = () => {
    setSkipOpen(false);
    setErrored(false);
    skip.mutate(query.id, { onError: () => setErrored(true) });
  };

  // Folded summary for already-resolved rows (defensive — the list shows pending).
  if (!pending && !errored) {
    const label =
      query.status === "answered"
        ? t("qa_answered_summary")
        : query.status === "skipped"
          ? t("qa_skipped_summary")
          : t("qa_resolved_summary");
    return (
      <div
        data-type="decision"
        className="rounded-card border border-divider bg-surface px-4 py-3 shadow-card [border-inline-start-width:3px]"
        style={{ borderInlineStartColor: borderVar(query.status, isT4, false) }}
      >
        <p className="font-ui text-xs uppercase tracking-wider text-p7">
          {query.triggerType} · {label}
        </p>
      </div>
    );
  }

  return (
    <div
      data-type="decision"
      className="rounded-card border border-divider bg-surface p-4 shadow-card [border-inline-start-width:3px]"
      style={{ borderInlineStartColor: borderVar(query.status, isT4, errored) }}
    >
      {/* Email-ref header */}
      <div className="flex items-center justify-between gap-2">
        <span className="section-label">
          {isT4 ? "T4 · " : ""}
          {t("qa_card_pending_label")}
        </span>
        {query.mailId && (
          <button
            type="button"
            onClick={openEmail}
            className="shrink-0 font-ui text-[10px] uppercase tracking-wider text-p9 hover:underline"
          >
            {t("qa_open_email")}
          </button>
        )}
      </div>

      {/* Question */}
      <p className="mt-2 font-body text-sm text-p10">{card?.questionText ?? query.question}</p>

      {/* Quick-option chips */}
      {chipOptions.length > 0 && (
        <div className="mt-3 flex flex-wrap gap-2">
          {chipOptions.map((opt) => {
            const on = selected.includes(opt.id);
            return (
              <button
                key={opt.id}
                type="button"
                aria-pressed={on}
                onClick={() => toggle(opt.id)}
                className={cn(
                  "rounded-chip px-3 py-1.5 font-ui text-xs transition-colors",
                  "focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
                  on ? "bg-p9 text-white" : "border border-divider text-p9 hover:bg-p4",
                )}
              >
                {opt.label}
              </button>
            );
          })}
          {hasViewEmail && query.mailId && (
            <button
              type="button"
              onClick={openEmail}
              className="rounded-chip border border-divider px-3 py-1.5 font-ui text-xs text-p9 hover:bg-p4"
            >
              {t("qa_open_email")}
            </button>
          )}
        </div>
      )}

      {/* Free-text note */}
      <textarea
        value={freeText}
        onChange={(e) => setFreeText(e.target.value.slice(0, 500))}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            submit();
          }
        }}
        rows={2}
        placeholder={t("qa_free_text_placeholder")}
        className="mt-3 w-full resize-none rounded-card border border-divider bg-p1 px-3 py-2 font-body text-sm text-p10 placeholder:text-p7 focus:outline-none focus:ring-1 focus:ring-p9"
      />

      {errored && (
        <p role="alert" className="mt-2 font-body text-xs text-amber">
          {t("qa_error_label")}
        </p>
      )}

      {/* Actions */}
      <div className="mt-3 flex items-center justify-between gap-2">
        <button
          type="button"
          onClick={() => setSkipOpen(true)}
          disabled={busy}
          className="rounded-chip px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p8 hover:bg-p4 disabled:opacity-40"
        >
          {card && card.subQuestions.length > 0 ? t("qa_skip_all") : t("qa_skip")}
        </button>
        {errored ? (
          <button
            type="button"
            onClick={() => setErrored(false)}
            className="hover:bg-amber/10 rounded-chip border border-amber px-4 py-1.5 font-ui text-xs uppercase tracking-wider text-amber"
          >
            {t("qa_retry")}
          </button>
        ) : (
          <button
            type="button"
            onClick={submit}
            disabled={!canSubmit}
            className="rounded-chip bg-p9 px-4 py-1.5 font-ui text-xs uppercase tracking-wider text-white transition-colors hover:bg-p10 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {t("qa_submit_reply")}
          </button>
        )}
      </div>

      <ConfirmDialog
        open={skipOpen}
        title={t("qa_skip_confirm_title")}
        body={t("qa_skip_confirm_body")}
        confirmLabel={t("qa_skip_confirm_cta")}
        destructive
        onConfirm={confirmSkip}
        onCancel={() => setSkipOpen(false)}
      />
    </div>
  );
}
