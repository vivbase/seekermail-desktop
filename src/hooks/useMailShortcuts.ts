// Keyboard shortcuts for the L0 mail stream (T038, F_G1 §4.5).
// Single-character shortcuts are guarded against input fields (dev/11 §2).
// Attaches to the document; cleaned up on unmount.
import { useCallback, useEffect, type RefObject } from "react";
import { useNavigate } from "react-router-dom";

import { useSelection } from "@/stores/selection";
import {
  useSetMailRead,
  useSetMailStarred,
  useArchiveMail,
  useDeleteMail,
} from "@/ipc/queries/mail";
import type { Thread } from "@shared/bindings";
import type { ThreadListHandle } from "@/components/mail/ThreadList";

interface UseMailShortcutsOptions {
  /** The ordered list of threads currently visible in the stream. */
  threads: Thread[];
  /** Ref to the ThreadList so we can call scrollToIndex. */
  listRef?: RefObject<ThreadListHandle | null>;
  /** Called when archive completes (triggers UndoToast in parent). */
  onArchived?: (id: string) => void;
  /** Called when delete completes (triggers UndoToast in parent). */
  onDeleted?: (id: string) => void;
}

/** Returns true when the current focus target is a text-input element. */
function isInputFocused(): boolean {
  const el = document.activeElement as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || el.isContentEditable;
}

export function useMailShortcuts({
  threads,
  listRef,
  onArchived,
  onDeleted,
}: UseMailShortcutsOptions) {
  const navigate = useNavigate();

  const selectedThreadId = useSelection((s) => s.selectedThreadId);
  const selectThread = useSelection((s) => s.selectThread);
  const toggleChecked = useSelection((s) => s.toggleChecked);

  const setMailRead = useSetMailRead();
  const setMailStarred = useSetMailStarred();
  const archiveMail = useArchiveMail();
  const deleteMail = useDeleteMail();

  /** Index of the currently focused/selected thread in the visible list. */
  const currentIndex = useCallback((): number => {
    if (!selectedThreadId) return -1;
    return threads.findIndex((t) => t.id === selectedThreadId);
  }, [selectedThreadId, threads]);

  /** Move selection to a given index, scrolling the virtual list if needed. */
  const moveTo = useCallback(
    (index: number) => {
      const clamped = Math.max(0, Math.min(threads.length - 1, index));
      const thread = threads[clamped];
      if (!thread) return;
      selectThread(thread.id);
      listRef?.current?.scrollToIndex(clamped);
    },
    [listRef, selectThread, threads],
  );

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      // Guard: do not fire when a text input has focus (dev/11 §2).
      if (isInputFocused()) return;

      const idx = currentIndex();
      const focused = idx >= 0 ? threads[idx] : (threads[0] ?? null);

      switch (e.key) {
        case "j":
        case "J":
          e.preventDefault();
          moveTo(idx < 0 ? 0 : idx + 1);
          break;

        case "k":
        case "K":
          e.preventDefault();
          moveTo(idx < 0 ? 0 : idx - 1);
          break;

        case "Enter":
          if (focused) {
            e.preventDefault();
            navigate(`/mail/${focused.id}`);
          }
          break;

        case "e":
        case "E":
          if (focused) {
            e.preventDefault();
            void archiveMail.mutate(focused.id, {
              onSuccess: () => onArchived?.(focused.id),
            });
          }
          break;

        case "#":
          if (focused) {
            e.preventDefault();
            void deleteMail.mutate(focused.id, {
              onSuccess: () => onDeleted?.(focused.id),
            });
          }
          break;

        case "s":
        case "S":
          if (focused) {
            e.preventDefault();
            void setMailStarred.mutate({
              mailId: focused.id,
              isStarred: !focused.isStarred,
            });
          }
          break;

        case "u":
        case "U":
          if (focused) {
            e.preventDefault();
            const isRead = focused.unreadCount === 0;
            void setMailRead.mutate({ mailId: focused.id, isRead: !isRead });
          }
          break;

        case "x":
        case "X":
          if (focused) {
            e.preventDefault();
            toggleChecked(focused.id);
          }
          break;

        case "c":
        case "C":
          e.preventDefault();
          navigate("/compose");
          break;

        case "Escape":
          // Clear selection/check on Escape
          selectThread(null);
          break;

        default:
          break;
      }
    },
    [
      archiveMail,
      currentIndex,
      deleteMail,
      moveTo,
      navigate,
      onArchived,
      onDeleted,
      selectThread,
      setMailRead,
      setMailStarred,
      threads,
      toggleChecked,
    ],
  );

  useEffect(() => {
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);
}
