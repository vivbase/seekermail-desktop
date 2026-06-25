// In-place AI reply draft card state (E1 inline draft flow). Client UI state
// ONLY (07 §5) — open/minimized + which mail/scope the card is drafting for.
// The draft DATA lives in TanStack Query (via the drafts.ts hooks), never here.
import { create } from "zustand";

import type { MailDetail } from "@shared/bindings";
import type { AiReplyScope } from "@/ipc/queries/drafts";

interface AiReplyCardState {
  /** Whether the growing draft card is mounted/visible for a mail. */
  open: boolean;
  /** Collapsed to the "Draft in progress" chip without losing the draft. */
  minimized: boolean;
  /** The mail being replied to; null when the card is closed. */
  mail: MailDetail | null;
  /** Sender-only ("reply") or sender + Cc ("reply-all"). */
  scope: AiReplyScope;
  /**
   * The receiving account's own address — excluded from the reply-all envelope
   * and carried through to the blank-compose fallback if generation fails.
   */
  ownEmail: string;
  /** Open (or re-open) the card to draft a reply to `mail` in `scope`. */
  openCard: (mail: MailDetail, scope: AiReplyScope, ownEmail?: string) => void;
  /** Collapse to the chip; the draft stays alive. */
  minimize: () => void;
  /** Expand the chip back to the full card. */
  resume: () => void;
  /** Close and clear — on send, discard, or escape to /compose. */
  close: () => void;
}

export const useAiReplyCard = create<AiReplyCardState>((set) => ({
  open: false,
  minimized: false,
  mail: null,
  scope: "reply",
  ownEmail: "",
  openCard: (mail, scope, ownEmail = "") =>
    set({ open: true, minimized: false, mail, scope, ownEmail }),
  minimize: () => set({ minimized: true }),
  resume: () => set({ minimized: false }),
  close: () => set({ open: false, minimized: false, mail: null, ownEmail: "" }),
}));
