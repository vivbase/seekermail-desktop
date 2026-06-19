// In-flight compose buffer (07 §5). Ephemeral draft text the user is typing. The
// backend persists drafts separately (T045); this is the live editor state.
import { create } from "zustand";

export interface ComposeBuffer {
  accountId: string | null;
  to: string;
  cc: string;
  bcc: string;
  subject: string;
  /**
   * Plain-text mirror of the editor content. Source of truth for validation,
   * autosave's `bodyText`, word counts, and the attachment-hint scan. Always
   * kept in sync with `bodyHtml` by the editor (T044, F_G4 §4.4).
   */
  body: string;
  /**
   * Rich-text (HTML) editor content. Becomes the `text/html` MIME part on send
   * and the persisted `body_html` on draft save. Empty string when the message
   * carries no formatting yet.
   */
  bodyHtml: string;
  /** Message-ID being replied to (drives threading on send). */
  inReplyTo: string | null;
  /** The persisted draft id once autosave has run (T045). */
  draftId: string | null;
  /** The originating `ai_drafts` row when seeded from an AI reply (T078, E1). */
  aiDraftId: string | null;
}

interface ComposeState extends ComposeBuffer {
  isOpen: boolean;
  /** True while a reply/forward quote is being composed (controls cc visibility). */
  ccVisible: boolean;
  /** True while `regenerate_draft` is in flight — Send stays disabled (T078). */
  aiRegenerating: boolean;
  open: (seed?: Partial<ComposeBuffer>) => void;
  update: (patch: Partial<ComposeBuffer>) => void;
  setCcVisible: (visible: boolean) => void;
  setAiRegenerating: (on: boolean) => void;
  reset: () => void;
}

const EMPTY: ComposeBuffer = {
  accountId: null,
  to: "",
  cc: "",
  bcc: "",
  subject: "",
  body: "",
  bodyHtml: "",
  inReplyTo: null,
  draftId: null,
  aiDraftId: null,
};

export const useCompose = create<ComposeState>((set) => ({
  ...EMPTY,
  isOpen: false,
  ccVisible: false,
  aiRegenerating: false,
  open: (seed) => set({ ...EMPTY, ...seed, isOpen: true, ccVisible: !!(seed?.cc || seed?.bcc) }),
  update: (patch) => set((s) => ({ ...s, ...patch })),
  setCcVisible: (ccVisible) => set({ ccVisible }),
  setAiRegenerating: (aiRegenerating) => set({ aiRegenerating }),
  reset: () => set({ ...EMPTY, isOpen: false, ccVisible: false, aiRegenerating: false }),
}));
