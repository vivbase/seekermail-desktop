// L2 reading-view keyboard shortcuts (T041, F_G3 §4.6).
// Attaches to document; cleans up on unmount. Guarded against input focus (dev/11 §2).
// Keys: r reply  a reply-all  f forward  e archive  # delete
//       u mark-unread  Esc/Backspace/ArrowLeft back  [ prev  ] next
//       . toggle-more-menu (i AI, v0.5+)
import { useCallback, useEffect } from "react";
import { useNavigate } from "react-router-dom";

import { useArchiveMail, useDeleteMail, useSetMailRead } from "@/ipc/queries/mail";
import { useAccounts } from "@/ipc/queries/accounts";
import type { MailDetail } from "@shared/bindings";

interface UseL2ShortcutsOptions {
  mail: MailDetail | null | undefined;
  /** Called when user presses . (period) to toggle the More menu. */
  onToggleMore?: () => void;
  /** Called when archive completes (caller shows UndoToast). */
  onArchived?: (id: string) => void;
  /** Called when delete completes (caller shows UndoToast). */
  onDeleted?: (id: string) => void;
}

/** Returns true when the event target is a text-entry element. */
function isInputFocused(): boolean {
  const el = document.activeElement as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || el.isContentEditable;
}

export function useL2Shortcuts({
  mail,
  onToggleMore,
  onArchived,
  onDeleted,
}: UseL2ShortcutsOptions) {
  const navigate = useNavigate();
  const archiveMail = useArchiveMail();
  const deleteMail = useDeleteMail();
  const setMailRead = useSetMailRead();

  // The receiving account is the default From-account on reply and is excluded
  // from the reply-all recipient list. Resolved by the compose route from state.
  const { data: accounts = [] } = useAccounts();
  const ownEmail = accounts.find((a) => a.id === mail?.accountId)?.email ?? "";

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      // Guard: do not fire while a text-input has focus (dev/11 §2).
      if (isInputFocused()) return;

      switch (e.key) {
        // Reply
        case "r":
        case "R":
          if (!mail) break;
          e.preventDefault();
          navigate("/compose", { state: { mode: "reply", mail, ownEmail } });
          break;

        // Reply all
        case "a":
        case "A":
          if (!mail) break;
          e.preventDefault();
          navigate("/compose", { state: { mode: "reply-all", mail, ownEmail } });
          break;

        // Forward
        case "f":
        case "F":
          if (!mail) break;
          e.preventDefault();
          navigate("/compose", { state: { mode: "forward", mail, ownEmail } });
          break;

        // Archive
        case "e":
        case "E":
          if (!mail) break;
          e.preventDefault();
          archiveMail.mutate(mail.id, {
            onSuccess: () => {
              onArchived?.(mail.id);
              navigate("/");
            },
          });
          break;

        // Delete
        case "#":
          if (!mail) break;
          e.preventDefault();
          deleteMail.mutate(mail.id, {
            onSuccess: () => {
              onDeleted?.(mail.id);
              navigate("/");
            },
          });
          break;

        // Mark unread + back
        case "u":
        case "U":
          if (!mail) break;
          e.preventDefault();
          setMailRead.mutate({ mailId: mail.id, isRead: false });
          navigate("/");
          break;

        // Back to list
        case "Escape":
        case "Backspace":
        case "ArrowLeft":
          e.preventDefault();
          navigate(-1);
          break;

        // Previous message in thread (placeholder — thread nav is single-message for now)
        case "[":
          e.preventDefault();
          navigate(-1);
          break;

        // Next message in thread (placeholder)
        case "]":
          e.preventDefault();
          // No next item in single-message view; no-op but consumed.
          break;

        // Toggle more menu
        case ".":
          e.preventDefault();
          onToggleMore?.();
          break;

        // AI reply — v0.5+ placeholder (consume key to prevent browser default)
        case "i":
        case "I":
          e.preventDefault();
          // AI reply not yet connected.
          break;

        default:
          break;
      }
    },
    [
      archiveMail,
      deleteMail,
      mail,
      navigate,
      onArchived,
      onDeleted,
      onToggleMore,
      ownEmail,
      setMailRead,
    ],
  );

  useEffect(() => {
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);
}
