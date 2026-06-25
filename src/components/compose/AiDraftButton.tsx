// Compose-time "AI Draft" affordance (analysis/57 §7) — the single AI-writing
// umbrella for compose. In Forward mode it captures an intent and writes a
// forwarding note above the quote; in New mode it writes a body from a one-line
// description. Reply / reply-all defer to the reading-view "AI Reply" (decision
// D3), so this renders nothing there. Compose is always manual-send — the
// low-risk analog of Manual mode — so the user reviews before sending.
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

import { useCompose } from "@/stores/compose";
import { useGenerateComposeDraft } from "@/ipc/queries/aiCompose";
import { forwardedExcerpt, insertNoteAboveQuote } from "@/lib/composeDraft";
import { plainTextToHtml } from "@/lib/richText";
import { showToast } from "@/components/ui/Toast";
import { cn } from "@/lib/cn";
import type { ComposeMode } from "./ComposeToolbar";

/** Forward intent presets (analysis/57 §3.2). Ids match the backend mapping. */
const INTENT_IDS = ["handle", "fyi", "review", "delegate", "records"] as const;
type IntentId = (typeof INTENT_IDS)[number];

const TONES = ["Formal", "Friendly", "Brief"] as const;
type Tone = (typeof TONES)[number];

/** E4 sensitive signal mirrored client-side: an amount drives the disclosure. */
const AMOUNT_RE = /[$€£¥]\s?\d|\b\d[\d,]*\.\d{2}\b/;

interface AiDraftButtonProps {
  mode: ComposeMode;
}

function SparkleIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        d="M8 1.5 9.5 6 14 7.5 9.5 9 8 13.5 6.5 9 2 7.5 6.5 6 8 1.5ZM13 2v2M14 3h-2"
      />
    </svg>
  );
}

export function AiDraftButton({ mode }: AiDraftButtonProps) {
  const { t } = useTranslation("compose");
  const navigate = useNavigate();
  const generate = useGenerateComposeDraft();

  const accountId = useCompose((s) => s.accountId);
  const to = useCompose((s) => s.to);
  const body = useCompose((s) => s.body);
  const update = useCompose((s) => s.update);
  const setAiRegenerating = useCompose((s) => s.setAiRegenerating);

  const [open, setOpen] = useState(false);
  const [intent, setIntent] = useState<IntentId | null>(null);
  const [note, setNote] = useState("");
  const [tone, setTone] = useState<Tone>("Friendly");
  const containerRef = useRef<HTMLDivElement>(null);

  const isNew = mode === "new";
  const isForward = mode === "forward";
  // D3: only Forward + New surface the compose-bar AI Draft.
  const enabledForMode = isNew || isForward;

  useEffect(() => {
    if (!open) return undefined;
    function onDocClick(e: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  if (!enabledForMode) return null;

  const excerpt = isForward ? forwardedExcerpt(body) : "";
  const sourceIsSensitive = isForward && AMOUNT_RE.test(excerpt);
  const recipientName = to
    .trim()
    .replace(/<[^>]*>/, "")
    .trim();
  const hasRecipient = to.trim().length > 0;
  const canGenerate = isNew ? note.trim().length > 0 : hasRecipient && intent !== null;

  async function runGenerate() {
    if (!canGenerate || !accountId) return;
    setAiRegenerating(true);
    try {
      const result = await generate.mutateAsync({
        accountId,
        mode,
        to: to.trim() || null,
        intent: isForward ? intent : null,
        note: note.trim() || null,
        tone,
        sourceExcerpt: excerpt || null,
      });
      const nextBody = insertNoteAboveQuote(body, result.body);
      update({ body: nextBody, bodyHtml: plainTextToHtml(nextBody) });
      setOpen(false);
      showToast(t("ai_draft_done"));
    } catch (err) {
      const e = err as { code?: string; detail?: string | null };
      if (e.code === "AI_PROVIDER_UNREACHABLE" && (e.detail ?? "").includes("not_configured")) {
        showToast(t("ai_draft_provider_missing"));
        void navigate("/settings/ai");
      } else {
        showToast(t("ai_draft_failed"));
      }
    } finally {
      setAiRegenerating(false);
    }
  }

  return (
    <div ref={containerRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        disabled={generate.isPending}
        aria-haspopup="dialog"
        aria-expanded={open}
        title={t("ai_draft_btn")}
        className={cn(
          "border-terra/40 flex items-center gap-1.5 rounded-chip border px-3 py-1.5",
          "font-ui text-[10px] uppercase tracking-wider text-p9 transition-colors",
          "hover:bg-p4 hover:text-p10 disabled:opacity-40",
          "focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
        )}
      >
        <SparkleIcon />
        {t("ai_draft_btn")}
      </button>

      {open && (
        <div
          role="dialog"
          aria-label={t("ai_draft_options")}
          className="absolute bottom-full end-0 mb-2 w-80 rounded-card border border-divider bg-surface p-4 shadow-card"
        >
          <p className="section-label mb-3">{t("ai_draft_title")}</p>

          {isForward &&
            (hasRecipient ? (
              <p className="mb-3 font-body text-xs text-p8">
                {t("ai_draft_writing_for", { name: recipientName })}
              </p>
            ) : (
              <p className="mb-3 font-body text-xs text-amber">{t("ai_draft_need_recipient")}</p>
            ))}

          {isForward && (
            <>
              <p className="section-label mb-2">{t("ai_draft_intent_label")}</p>
              <div className="mb-3 flex flex-wrap gap-2">
                {INTENT_IDS.map((id) => (
                  <button
                    key={id}
                    type="button"
                    onClick={() => setIntent(id)}
                    aria-pressed={intent === id}
                    className={cn(
                      "rounded-chip px-2.5 py-1 font-ui text-[10px] uppercase tracking-wider transition-colors",
                      intent === id
                        ? "bg-p9 text-white"
                        : "border border-divider text-p8 hover:bg-p4 hover:text-p10",
                    )}
                  >
                    {t(`ai_draft_intent_${id}`)}
                  </button>
                ))}
              </div>
            </>
          )}

          <textarea
            value={note}
            onChange={(e) => setNote(e.target.value)}
            rows={2}
            placeholder={isNew ? t("ai_draft_about_placeholder") : t("ai_draft_note_placeholder")}
            className="mb-3 w-full resize-none rounded-chip border border-divider bg-parchment p-2 font-body text-xs text-p10 placeholder:text-p7 focus:outline-none focus:ring-1 focus:ring-p9"
          />

          <p className="section-label mb-2">{t("ai_draft_tone_label")}</p>
          <div className="mb-3 flex gap-2">
            {TONES.map((toneOption) => (
              <button
                key={toneOption}
                type="button"
                onClick={() => setTone(toneOption)}
                aria-pressed={tone === toneOption}
                className={cn(
                  "flex-1 rounded-chip px-2 py-1 font-ui text-[10px] uppercase tracking-wider transition-colors",
                  tone === toneOption
                    ? "bg-p9 text-white"
                    : "border border-divider text-p8 hover:bg-p4 hover:text-p10",
                )}
              >
                {t(`ai_draft_tone_${toneOption.toLowerCase()}`)}
              </button>
            ))}
          </div>

          {sourceIsSensitive && (
            <p className="bg-amber/10 mb-3 rounded-chip px-2.5 py-2 font-body text-[11px] leading-snug text-p8">
              {t("ai_draft_disclosure")}
            </p>
          )}

          <div className="flex justify-end gap-2">
            <button
              type="button"
              onClick={() => setOpen(false)}
              className="rounded-chip px-3 py-1.5 font-ui text-[10px] uppercase tracking-wider text-p7 hover:bg-p4 hover:text-p10"
            >
              {t("ai_draft_cancel")}
            </button>
            <button
              type="button"
              onClick={() => void runGenerate()}
              disabled={!canGenerate || generate.isPending}
              className="rounded-chip bg-p9 px-4 py-1.5 font-ui text-[10px] font-semibold uppercase tracking-wider text-white hover:bg-p10 disabled:opacity-40"
            >
              {generate.isPending ? t("ai_draft_writing") : t("ai_draft_generate")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
