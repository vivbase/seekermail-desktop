// E1 manual AI reply trigger (T078, F_E1 §4.1/§4.4/§5). Lives in the L2 action
// bar (MailToolbar). Three states: idle → "AI Reply", drafting → spinner (text
// "AI is drafting…" only after 1.5 s), failed → amber icon + "Regenerate" as
// the retry affordance. Navigation/toasts are handled by useRequestAiReply.
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { MailDetail } from "@shared/bindings";

import { useRequestAiReply } from "@/ipc/queries/drafts";
import { cn } from "@/lib/cn";

/** Spinner-text threshold (F_E1 §4.4): under 1.5 s only the spinner shows. */
const DRAFTING_TEXT_DELAY_MS = 1500;

interface AiReplyButtonProps {
  mail: MailDetail;
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

function SpinnerIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      className="animate-spin"
      aria-hidden="true"
    >
      <path strokeLinecap="round" d="M8 1.5A6.5 6.5 0 1 1 1.5 8" />
    </svg>
  );
}

function RetryIcon() {
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
        d="M2.5 8a5.5 5.5 0 1 1 1.6 3.9M2.5 12V8.8h3.2"
      />
    </svg>
  );
}

export function AiReplyButton({ mail }: AiReplyButtonProps) {
  const { t } = useTranslation("aiDrafts");
  const request = useRequestAiReply();

  /** Tri-state per F_E1 §5; "drafting" derives from the mutation in-flight flag. */
  const [failed, setFailed] = useState(false);
  const isDrafting = request.isPending;

  const [showDraftingText, setShowDraftingText] = useState(false);
  useEffect(() => {
    if (!isDrafting) {
      setShowDraftingText(false);
      return;
    }
    const timer = setTimeout(() => setShowDraftingText(true), DRAFTING_TEXT_DELAY_MS);
    return () => clearTimeout(timer);
  }, [isDrafting]);

  const label = isDrafting
    ? showDraftingText
      ? t("e1_ai_drafting")
      : t("e1_ai_reply_btn")
    : failed
      ? t("e1_regenerate")
      : t("e1_ai_reply_btn");

  function handleClick() {
    setFailed(false);
    request.mutate({ mail }, { onError: () => setFailed(true) });
  }

  return (
    <button
      type="button"
      onClick={handleClick}
      disabled={isDrafting}
      aria-label={label}
      aria-busy={isDrafting}
      title={label}
      className={cn(
        "flex items-center gap-1.5 rounded-chip px-3 py-1.5 font-ui text-xs uppercase tracking-wider transition-colors",
        "focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
        failed ? "hover:bg-amber/10 text-amber" : "text-p9 hover:bg-p4 hover:text-p10",
        "disabled:opacity-60",
      )}
    >
      {isDrafting ? <SpinnerIcon /> : failed ? <RetryIcon /> : <SparkleIcon />}
      <span className="hidden sm:inline">{label}</span>
    </button>
  );
}
