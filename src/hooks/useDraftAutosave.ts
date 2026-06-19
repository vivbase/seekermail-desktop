// Debounced draft autosave hook (T045). Watches the compose store and writes to
// the backend via useSaveDraft after a 1.5 s debounce period when meaningful
// content is present. Writes the returned draft id back into the store so
// subsequent saves perform an UPDATE rather than INSERT.

import { useCallback, useEffect, useRef, useState } from "react";
import { useCompose } from "@/stores/compose";
import { useSaveDraft } from "@/ipc/queries/drafts";
import type { SaveDraftParams } from "@shared/bindings";
import { parseRecipients } from "@/lib/composeValidation";
import { isHtmlBlank } from "@/lib/richText";

// ── Constants ────────────────────────────────────────────────────────────────

/** Debounce delay in ms before triggering a save after the last content change. */
const DEBOUNCE_MS = 1_500;

// ── Types ────────────────────────────────────────────────────────────────────

export type AutosaveStatus = "idle" | "saving" | "saved" | "error";

export interface UseDraftAutosaveReturn {
  status: AutosaveStatus;
  /** Manually trigger a save right now (bypasses the debounce). */
  saveNow: () => Promise<void>;
}

// ── Hook ─────────────────────────────────────────────────────────────────────

export function useDraftAutosave(): UseDraftAutosaveReturn {
  const [autosaveStatus, setAutosaveStatus] = useState<AutosaveStatus>("idle");

  // Read compose state slices individually to avoid unnecessary re-renders.
  const accountId = useCompose((s) => s.accountId);
  const to = useCompose((s) => s.to);
  const cc = useCompose((s) => s.cc);
  const subject = useCompose((s) => s.subject);
  const body = useCompose((s) => s.body);
  const bodyHtml = useCompose((s) => s.bodyHtml);
  const inReplyTo = useCompose((s) => s.inReplyTo);
  const draftId = useCompose((s) => s.draftId);
  const update = useCompose((s) => s.update);

  const { mutateAsync: saveDraft } = useSaveDraft();

  const isMountedRef = useRef(true);
  const debounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    isMountedRef.current = true;
    return () => {
      isMountedRef.current = false;
      if (debounceTimerRef.current !== null) {
        clearTimeout(debounceTimerRef.current);
      }
    };
  }, []);

  /** True when the compose buffer contains meaningful content worth persisting. */
  function hasMeaningfulContent(): boolean {
    return to.trim().length > 0 || subject.trim().length > 0 || body.trim().length > 0;
  }

  /** Build SaveDraftParams from the current store state. */
  function buildParams(): SaveDraftParams | null {
    if (!accountId) return null;
    return {
      id: draftId,
      accountId,
      to: parseRecipients(to),
      cc: parseRecipients(cc),
      subject,
      bodyText: body,
      bodyHtml: isHtmlBlank(bodyHtml) ? null : bodyHtml,
      inReplyTo,
    };
  }

  const performSave = useCallback(async (): Promise<void> => {
    if (!hasMeaningfulContent()) return;
    const params = buildParams();
    if (!params) return;

    if (isMountedRef.current) setAutosaveStatus("saving");
    try {
      const draft = await saveDraft(params);
      if (isMountedRef.current) {
        // Write the returned id back so future saves are updates.
        update({ draftId: draft.id });
        setAutosaveStatus("saved");
      }
    } catch {
      if (isMountedRef.current) setAutosaveStatus("error");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accountId, to, cc, subject, body, bodyHtml, inReplyTo, draftId, saveDraft, update]);

  // Debounce: schedule a save 1.5 s after any content change.
  useEffect(() => {
    if (debounceTimerRef.current !== null) {
      clearTimeout(debounceTimerRef.current);
    }
    if (!hasMeaningfulContent()) return;

    debounceTimerRef.current = setTimeout(() => {
      void performSave();
    }, DEBOUNCE_MS);

    return () => {
      if (debounceTimerRef.current !== null) {
        clearTimeout(debounceTimerRef.current);
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [to, subject, body, bodyHtml, cc, accountId]);

  return {
    status: autosaveStatus,
    saveNow: performSave,
  };
}
