// Draft-save status badge (T045, F_G4 §4.11). Reads the autosave status from the
// parent and renders a brief status message with an aria-live region so screen
// readers are notified without interrupting the editing flow (dev/11 §5).

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AutosaveStatus } from "@/hooks/useDraftAutosave";
import { cn } from "@/lib/cn";

interface DraftSaveBadgeProps {
  status: AutosaveStatus;
}

/** Duration in ms the "saved" message remains visible before fading out. */
const SAVED_DISPLAY_MS = 3_000;

export function DraftSaveBadge({ status }: DraftSaveBadgeProps) {
  const { t } = useTranslation("compose");

  // Control visibility independently from the status so we can fade the "saved"
  // state out after a delay while the status remains "saved" in the parent.
  const [visible, setVisible] = useState(false);
  const hideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Respect prefers-reduced-motion: skip fade, hide immediately.
  const prefersReducedMotion =
    typeof window !== "undefined" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  useEffect(() => {
    if (hideTimerRef.current !== null) clearTimeout(hideTimerRef.current);

    if (status === "idle") {
      setVisible(false);
      return;
    }

    setVisible(true);

    if (status === "saved") {
      hideTimerRef.current = setTimeout(
        () => setVisible(false),
        prefersReducedMotion ? 0 : SAVED_DISPLAY_MS,
      );
    }

    return () => {
      if (hideTimerRef.current !== null) clearTimeout(hideTimerRef.current);
    };
  }, [status, prefersReducedMotion]);

  const label =
    status === "saving"
      ? t("draft_saving")
      : status === "saved"
        ? t("draft_saved")
        : status === "error"
          ? t("draft_save_failed")
          : "";

  return (
    // aria-live="polite" so assistive tech announces status changes without
    // interrupting the user (dev/11 §5).
    <div aria-live="polite" aria-atomic="true" className="h-5">
      <span
        className={cn(
          "font-ui text-[10px] uppercase tracking-wider transition-opacity duration-500",
          visible ? "opacity-100" : "opacity-0",
          status === "saved" && "text-green",
          status === "saving" && "text-p7",
          status === "error" && "text-red",
        )}
      >
        {label}
      </span>
    </div>
  );
}
