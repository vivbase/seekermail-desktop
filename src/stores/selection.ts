// Selection state (07 §5). Holds only IDS — the selected entities' DATA is read
// from TanStack Query, never copied here.
import { create } from "zustand";

interface SelectionState {
  selectedAccountId: string | null;
  selectedThreadId: string | null;
  selectedMailId: string | null;
  /** Multi-select set for bulk actions in the L0 stream (T038). */
  checkedThreadIds: Set<string>;
  /**
   * Per-mail scroll position memory (T041, F_G3 §4.7).
   * Key: mailId. Value: scrollTop px. Written on L2 unmount, read on mount.
   */
  mailScrollPositions: Map<string, number>;
  /**
   * D1 body highlight (T071, F_D1 §5). The `originalText` excerpt of the risk
   * item the user clicked in the Legal sidebar; MailBody wraps its first
   * occurrence in `<mark class="legal-highlight">`. Null = no highlight.
   */
  legalHighlightText: string | null;
  selectAccount: (id: string | null) => void;
  selectThread: (id: string | null) => void;
  selectMail: (id: string | null) => void;
  toggleChecked: (id: string) => void;
  setChecked: (ids: string[]) => void;
  clearChecked: () => void;
  isChecked: (id: string) => boolean;
  /** Persist the scroll position for a mail (called on L2 unmount). */
  setScrollPosition: (mailId: string, pos: number) => void;
  /** Set (or clear with null) the D1 legal body highlight. */
  setLegalHighlight: (text: string | null) => void;
  clear: () => void;
}

export const useSelection = create<SelectionState>((set, get) => ({
  selectedAccountId: null,
  selectedThreadId: null,
  selectedMailId: null,
  checkedThreadIds: new Set<string>(),
  mailScrollPositions: new Map<string, number>(),
  legalHighlightText: null,
  selectAccount: (id) => set({ selectedAccountId: id }),
  selectThread: (id) => set({ selectedThreadId: id }),
  // Switching mail always drops the previous mail's legal highlight (T071 §3.3).
  selectMail: (id) => set({ selectedMailId: id, legalHighlightText: null }),
  toggleChecked: (id) =>
    set((s) => {
      const next = new Set(s.checkedThreadIds);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return { checkedThreadIds: next };
    }),
  setChecked: (ids) => set({ checkedThreadIds: new Set(ids) }),
  clearChecked: () => set({ checkedThreadIds: new Set<string>() }),
  isChecked: (id) => get().checkedThreadIds.has(id),
  setScrollPosition: (mailId, pos) =>
    set((s) => {
      const next = new Map(s.mailScrollPositions);
      next.set(mailId, pos);
      return { mailScrollPositions: next };
    }),
  setLegalHighlight: (text) => set({ legalHighlightText: text }),
  clear: () =>
    set({
      selectedAccountId: null,
      selectedThreadId: null,
      selectedMailId: null,
      checkedThreadIds: new Set<string>(),
      legalHighlightText: null,
    }),
}));
