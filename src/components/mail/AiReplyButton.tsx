// E1 manual AI reply trigger (T078, F_E1 §4.1/§4.4). Lives in the L2 action bar
// (MailToolbar). For a single-recipient mail it is a plain button that opens the
// in-place AI reply draft card (AiReplyDraftCard) to draft a sender-only reply.
// When the thread has other recipients it becomes a menu button: a single click
// opens a menu with "AI Reply" (sender) and "AI Reply All" (sender + Cc, the
// same set as a manual Reply all), so the user picks the scope before anything
// is generated. Generation, editing and sending all happen inside the card —
// this component is just the entry point that opens it via the aiReplyCard store.
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import type { MailDetail } from "@shared/bindings";

import { type AiReplyScope } from "@/ipc/queries/drafts";
import { useAiReplyCard } from "@/stores/aiReplyCard";
import { buildReplySeed, buildReplyAllSeed } from "@/lib/quoteBuilder";
import { cn } from "@/lib/cn";

interface AiReplyButtonProps {
  mail: MailDetail;
  /** Receiving account address — excluded from the reply-all recipient list. */
  ownEmail?: string;
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

function CaretIcon() {
  return (
    <svg
      width="10"
      height="10"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      aria-hidden="true"
    >
      <path strokeLinecap="round" strokeLinejoin="round" d="M4 6l4 4 4-4" />
    </svg>
  );
}

export function AiReplyButton({ mail, ownEmail = "" }: AiReplyButtonProps) {
  const { t } = useTranslation("aiDrafts");
  const openCard = useAiReplyCard((s) => s.openCard);

  // Reply-all is offered only when it would reach recipients a plain reply would
  // not — i.e. the reply-all envelope differs from the sender-only one. Reuses
  // the exact builders the manual reply path uses, so the two stay in lock-step.
  const canReplyAll = useMemo(
    () => buildReplyAllSeed(mail, ownEmail).to !== buildReplySeed(mail).to,
    [mail, ownEmail],
  );

  const [menuOpen, setMenuOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!menuOpen) return;
    function onDocClick(e: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setMenuOpen(false);
    }
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [menuOpen]);

  const label = t("e1_ai_reply_btn");

  function runReply(scope: AiReplyScope) {
    setMenuOpen(false);
    openCard(mail, scope, ownEmail);
  }

  const triggerClass = cn(
    "flex items-center gap-1.5 rounded-chip px-3 py-1.5 font-ui text-xs uppercase tracking-wider transition-colors",
    "text-p9 hover:bg-p4 hover:text-p10",
    "focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
  );

  // Single-recipient mail: a plain button drafts a sender-only reply directly —
  // a reply-all choice would be redundant when there is no one else to add.
  if (!canReplyAll) {
    return (
      <button
        type="button"
        onClick={() => runReply("reply")}
        aria-label={label}
        title={label}
        className={triggerClass}
      >
        <SparkleIcon />
        <span className="hidden sm:inline">{label}</span>
      </button>
    );
  }

  // Multi-recipient mail: a single click opens a menu so the user explicitly
  // picks AI Reply (sender) or AI Reply All (sender + Cc) before any drafting
  // begins — nothing is generated until a choice is made.
  return (
    <div ref={containerRef} className="relative flex items-center">
      <button
        type="button"
        onClick={() => setMenuOpen((o) => !o)}
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        aria-label={t("e1_ai_reply_options")}
        title={label}
        className={triggerClass}
      >
        <SparkleIcon />
        <span className="hidden sm:inline">{label}</span>
        <CaretIcon />
      </button>

      {menuOpen && (
        <div
          role="menu"
          className="absolute bottom-full start-0 mb-1 min-w-[176px] rounded-card border border-divider bg-surface py-1 shadow-card"
        >
          <button
            role="menuitem"
            type="button"
            onClick={() => runReply("reply")}
            className="flex w-full items-center gap-2 px-3 py-2 font-ui text-xs text-p8 hover:bg-p4 hover:text-p10 focus:outline-none focus-visible:bg-p4"
          >
            <SparkleIcon />
            {t("e1_ai_reply_btn")}
          </button>
          <button
            role="menuitem"
            type="button"
            onClick={() => runReply("reply-all")}
            className="flex w-full items-center gap-2 px-3 py-2 font-ui text-xs text-p8 hover:bg-p4 hover:text-p10 focus:outline-none focus-visible:bg-p4"
          >
            <SparkleIcon />
            {t("e1_ai_reply_all")}
          </button>
        </div>
      )}
    </div>
  );
}
