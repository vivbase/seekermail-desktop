// E1 inline AI reply — the in-place "growing draft card" (analysis: in-place
// draft flow). Replaces the navigate-to-/compose hand-off: the reply is drafted
// and edited right in the reading view, above the toolbar. Opened by
// AiReplyButton via the aiReplyCard store; mounted once by the mail-detail route.
//
// Flow: open → request_ai_reply (one-shot) with a staged skeleton → the result
// is revealed by typing it in → the user edits / switches tone (regenerate_draft
// with an instruction) / regenerates / sends (approve_draft) / discards, or
// escapes to the full /compose editor. It reuses the SAME draft object the
// Pending review surface shows — one draft, not two.
import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";

import type { AiDraft } from "@shared/bindings";

import {
  buildAiComposeSeed,
  useApproveDraft,
  useDiscardDraft,
  useGenerateAiReplyInline,
  useRegenerateDraft,
  useUpdateDraftBody,
} from "@/ipc/queries/drafts";
import { useAiReplyCard } from "@/stores/aiReplyCard";
import { showToast } from "@/components/ui/Toast";
import { markdownToPlainText } from "@/lib/markdown";
import { cn } from "@/lib/cn";

type Phase = "generating" | "ready";
type Tone = "default" | "concise" | "warmer" | "formal";

/** Pill labels shown while the one-shot request runs. */
const STAGE_KEYS = ["e1_stage_reading", "e1_stage_recalling", "e1_stage_drafting"] as const;

const TONES: Tone[] = ["default", "concise", "warmer", "formal"];
const TONE_LABEL_KEY: Record<Tone, string> = {
  default: "e1_tone_default",
  concise: "e1_tone_concise",
  warmer: "e1_tone_warmer",
  formal: "e1_tone_formal",
};

/** Tone → English instruction appended to regeneration (model prompt, not UI). */
const TONE_INSTRUCTION: Record<Tone, string | undefined> = {
  default: undefined,
  concise: "Rewrite the reply more concisely while keeping the key points.",
  warmer: "Rewrite the reply in a warmer, friendlier tone.",
  formal: "Rewrite the reply in a more formal, professional tone.",
};

const TYPE_STEP_MS = 18;
const STAGE_STEP_MS = 850;

function SparkleIcon() {
  return (
    <svg width="15" height="15" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
      <path d="M8 1.5 9.5 6 14 7.5 9.5 9 8 13.5 6.5 9 2 7.5 6.5 6 8 1.5Z" />
    </svg>
  );
}

export function AiReplyDraftCard() {
  const { t } = useTranslation("aiDrafts");
  const navigate = useNavigate();

  const open = useAiReplyCard((s) => s.open);
  const minimized = useAiReplyCard((s) => s.minimized);
  const mail = useAiReplyCard((s) => s.mail);
  const scope = useAiReplyCard((s) => s.scope);
  const ownEmail = useAiReplyCard((s) => s.ownEmail);
  const minimize = useAiReplyCard((s) => s.minimize);
  const resume = useAiReplyCard((s) => s.resume);
  const close = useAiReplyCard((s) => s.close);

  const generate = useGenerateAiReplyInline();
  const regenerate = useRegenerateDraft();
  const approve = useApproveDraft();
  const updateBody = useUpdateDraftBody();
  const discard = useDiscardDraft();

  const [draft, setDraft] = useState<AiDraft | null>(null);
  const [phase, setPhase] = useState<Phase>("generating");
  const [body, setBody] = useState("");
  const [edited, setEdited] = useState(false);
  const [tone, setTone] = useState<Tone>("default");
  const [stageIdx, setStageIdx] = useState(0);

  const typeTimer = useRef<number | null>(null);
  const stageTimer = useRef<number | null>(null);
  /** The mail id we have already kicked generation off for (guards double-fire). */
  const startedFor = useRef<string | null>(null);

  const clearTimers = useCallback(() => {
    if (typeTimer.current !== null) window.clearTimeout(typeTimer.current);
    if (stageTimer.current !== null) window.clearTimeout(stageTimer.current);
    typeTimer.current = null;
    stageTimer.current = null;
  }, []);

  /** Reveal the one-shot result by typing it in (the "writes itself" effect). */
  const typeIn = useCallback((text: string) => {
    let i = 0;
    const step = () => {
      i += 2;
      setBody(text.slice(0, i));
      if (i < text.length) {
        typeTimer.current = window.setTimeout(step, TYPE_STEP_MS);
      } else {
        setBody(text);
      }
    };
    step();
  }, []);

  // Generate a draft when the card opens for a mail (once per mail id).
  useEffect(() => {
    if (!open || !mail) return;
    if (startedFor.current === mail.id) return;
    startedFor.current = mail.id;

    clearTimers();
    setPhase("generating");
    setDraft(null);
    setBody("");
    setEdited(false);
    setTone("default");

    const cycle = (idx: number) => {
      setStageIdx(idx);
      if (idx < STAGE_KEYS.length - 1) {
        stageTimer.current = window.setTimeout(() => cycle(idx + 1), STAGE_STEP_MS);
      }
    };
    cycle(0);

    generate.mutate(
      { mailId: mail.id },
      {
        onSuccess: (d) => {
          clearTimers();
          setDraft(d);
          setPhase("ready");
          typeIn(markdownToPlainText(d.bodyCurrent));
        },
        onError: (err) => {
          clearTimers();
          startedFor.current = null;
          // F_E1 §4.4: an AI reply failure must never block the user. A missing
          // provider routes to AI settings; any other error falls back to a
          // blank reply in the full compose editor — which is exactly what the
          // toast promises ("opened a blank draft instead").
          const notConfigured =
            err.code === "AI_PROVIDER_UNREACHABLE" && (err.detail ?? "").includes("not_configured");
          const failedMail = mail;
          close();
          if (notConfigured) {
            showToast(t("toast_ai_provider_not_configured"));
            void navigate("/settings/ai");
            return;
          }
          showToast(t("toast_ai_draft_failed"));
          if (failedMail) {
            void navigate("/compose", {
              state: { mode: scope, mail: failedMail, ownEmail },
            });
          }
        },
      },
    );
    // generate/close/t/typeIn/clearTimers are stable for the lifetime of an open card.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, mail?.id]);

  // Reset local state whenever the card is fully closed.
  useEffect(() => {
    if (open) return;
    clearTimers();
    startedFor.current = null;
    setDraft(null);
    setBody("");
    setPhase("generating");
    setEdited(false);
  }, [open, clearTimers]);

  // Tear down timers on unmount.
  useEffect(() => clearTimers, [clearTimers]);

  if (!open || !mail) return null;

  // ── Actions ────────────────────────────────────────────────────────────────

  const runRegenerate = (nextTone: Tone) => {
    if (!draft) return;
    clearTimers();
    setTone(nextTone);
    setEdited(false);
    setPhase("generating");
    setStageIdx(STAGE_KEYS.length - 1); // "drafting"
    regenerate.mutate(
      { id: draft.id, instruction: TONE_INSTRUCTION[nextTone] },
      {
        onSuccess: (d) => {
          setDraft(d);
          setPhase("ready");
          typeIn(markdownToPlainText(d.bodyCurrent));
        },
        // A failed regeneration must not strand the card in the "generating"
        // skeleton — restore the existing draft so the user can retry or edit.
        onError: () => {
          setPhase("ready");
        },
      },
    );
  };

  const handleSend = async () => {
    if (!draft) return;
    try {
      if (edited) {
        await updateBody.mutateAsync({ id: draft.id, bodyCurrent: body });
      }
      await approve.mutateAsync(draft.id);
      showToast(t("toast_draft_sent"));
      close();
    } catch {
      // approve / updateBody surface their own error toasts — keep the card open.
    }
  };

  const handleDiscard = () => {
    if (draft) discard.mutate({ id: draft.id });
    showToast(t("toast_draft_discarded"));
    close();
  };

  const handleOpenFull = () => {
    if (!draft) return;
    const seed = buildAiComposeSeed(draft);
    seed.body = body; // carry the current (possibly edited) text
    seed.inReplyTo = mail.id;
    close();
    void navigate("/compose", { state: { mode: scope, aiSeed: seed } });
  };

  // ── Minimized chip ─────────────────────────────────────────────────────────

  if (minimized) {
    return (
      <div className="pointer-events-none fixed inset-x-0 bottom-24 z-40 flex justify-end px-6">
        <button
          type="button"
          onClick={resume}
          className="pointer-events-auto flex items-center gap-2.5 rounded-full bg-p10 px-4 py-2.5 text-white shadow-card transition-transform hover:scale-[1.02] focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
        >
          <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-green" aria-hidden="true" />
          <span className="font-ui text-xs font-medium">{t("e1_chip_in_progress")}</span>
          <span className="font-ui text-[10px] uppercase tracking-wider text-amber">
            {t("e1_chip_resume")}
          </span>
        </button>
      </div>
    );
  }

  // ── Full card ──────────────────────────────────────────────────────────────

  const isGenerating = phase === "generating";
  const stageLabel = isGenerating
    ? t(STAGE_KEYS[stageIdx] ?? "e1_stage_drafting")
    : edited
      ? t("e1_stage_editing")
      : t("e1_stage_ready");
  const busy = generate.isPending || regenerate.isPending;

  return (
    <div className="pointer-events-none fixed inset-x-0 bottom-24 z-40 flex justify-center px-4">
      <div
        className="pointer-events-auto flex w-full max-w-[560px] flex-col overflow-hidden rounded-card border border-divider bg-surface shadow-card transition-[max-height] duration-500 ease-out"
        style={{ maxHeight: isGenerating ? 188 : 460 }}
        role="dialog"
        aria-label={t("e1_card_title")}
      >
        {/* Header */}
        <div className="flex items-center gap-3 px-4 py-3">
          <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-chip bg-gradient-to-br from-terra to-amber text-white">
            <SparkleIcon />
          </span>
          <div className="min-w-0">
            <div className="font-ui text-[13px] font-semibold text-p10">{t("e1_card_title")}</div>
            <div className="font-mono text-[10px] text-p7">{t("e1_card_assistant")}</div>
          </div>
          <span
            className={cn(
              "ms-auto rounded-full px-2.5 py-1 font-ui text-[10.5px] font-semibold tracking-wide transition-colors",
              isGenerating || edited ? "bg-terra/10 text-terra" : "bg-green/12 text-green",
            )}
          >
            {stageLabel}
          </span>
          {!isGenerating && (
            <button
              type="button"
              onClick={minimize}
              aria-label={t("e1_card_minimize")}
              title={t("e1_card_minimize")}
              className="shrink-0 rounded-chip p-1 text-p7 transition-colors hover:bg-p4 hover:text-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
            >
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true">
                <path
                  d="M4 6l4 4 4-4"
                  stroke="currentColor"
                  strokeWidth="1.75"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </svg>
            </button>
          )}
        </div>

        {/* Recipient / subject */}
        {!isGenerating && draft && (
          <div className="px-4">
            <div className="flex items-baseline gap-2 border-t border-divider py-1.5">
              <span className="w-16 shrink-0 font-ui text-[9.5px] font-semibold uppercase tracking-wider text-p7">
                {t("draft_to_label")}
              </span>
              <span className="truncate font-ui text-xs text-p9">{draft.toAddr.email}</span>
            </div>
            <div className="flex items-baseline gap-2 border-t border-divider py-1.5">
              <span className="w-16 shrink-0 font-ui text-[9.5px] font-semibold uppercase tracking-wider text-p7">
                {t("e1_card_subject")}
              </span>
              <span className="truncate font-ui text-xs text-p9">{draft.subject}</span>
            </div>
          </div>
        )}

        {/* Body */}
        <div className="min-h-0 flex-1 px-4 py-3">
          {isGenerating ? (
            <div className="space-y-3" aria-hidden="true">
              <div className="h-2.5 w-[94%] animate-pulse rounded bg-p4" />
              <div className="h-2.5 w-[99%] animate-pulse rounded bg-p4" />
              <div className="h-2.5 w-[84%] animate-pulse rounded bg-p4" />
              <div className="h-2.5 w-[46%] animate-pulse rounded bg-p4" />
            </div>
          ) : (
            <textarea
              value={body}
              onChange={(e) => {
                setBody(e.target.value);
                setEdited(true);
              }}
              placeholder={t("e1_card_placeholder")}
              className="h-40 w-full resize-none rounded-chip bg-transparent p-1 font-body text-sm leading-relaxed text-p10 outline-none placeholder:text-p7"
            />
          )}
        </div>

        {/* Footer */}
        {!isGenerating && (
          <div className="flex flex-wrap items-center gap-2 border-t border-divider px-4 py-2.5">
            <button
              type="button"
              onClick={handleSend}
              disabled={approve.isPending || updateBody.isPending}
              className="rounded-chip bg-terra px-4 py-1.5 font-ui text-xs font-medium uppercase tracking-wider text-white transition-colors hover:bg-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9 disabled:opacity-60"
            >
              {t("e1_card_send")}
            </button>
            <button
              type="button"
              onClick={() => runRegenerate(tone)}
              disabled={busy}
              className="rounded-chip border border-divider px-3 py-1.5 font-ui text-xs font-medium uppercase tracking-wider text-p9 transition-colors hover:bg-p4 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9 disabled:opacity-60"
            >
              {t("e1_regenerate")}
            </button>

            <div className="flex items-center gap-1.5">
              {TONES.map((tn) => (
                <button
                  key={tn}
                  type="button"
                  onClick={() => runRegenerate(tn)}
                  disabled={busy}
                  aria-pressed={tone === tn}
                  className={cn(
                    "rounded-full border px-2.5 py-1 font-ui text-[10.5px] font-medium transition-colors disabled:opacity-60",
                    tone === tn
                      ? "bg-terra/10 border-terra text-terra"
                      : "border-divider text-p8 hover:border-terra hover:text-terra",
                  )}
                >
                  {t(TONE_LABEL_KEY[tn])}
                </button>
              ))}
            </div>

            <button
              type="button"
              onClick={handleOpenFull}
              className="ms-auto font-ui text-[11px] text-p8 underline-offset-2 transition-colors hover:text-p10 hover:underline focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
            >
              {t("draft_open_compose")}
            </button>
            <button
              type="button"
              onClick={handleDiscard}
              disabled={discard.isPending}
              className="hover:bg-red/10 rounded-chip px-2.5 py-1.5 font-ui text-xs font-medium uppercase tracking-wider text-p7 transition-colors hover:text-red focus:outline-none focus-visible:ring-2 focus-visible:ring-p9 disabled:opacity-60"
            >
              {t("e1_card_discard")}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
