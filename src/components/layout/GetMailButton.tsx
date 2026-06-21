// Icon-only "Get Mail" control in the nav rail (07 §3). A pinwheel that spins
// while a manual sync runs — no permanent text; the label shows only as a hover
// tooltip. Clicking force-syncs every active account via `trigger_sync` (the same
// path IMAP IDLE pokes automatically). The spin is non-blocking: it animates the
// icon in place and never covers the UI. It stops on the sync:complete / sync:error
// event, or a fallback timeout off-Tauri where no events fire.
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { isTauri } from "@/ipc/client";
import { useEvent } from "@/ipc/events";
import { useAccounts, useTriggerSync } from "@/ipc/queries/accounts";
import { cn } from "@/lib/cn";

/** Spinner hard-stop so a missing event can never leave the pinwheel spinning. */
const SPIN_FALLBACK_MS_TAURI = 20_000;
const SPIN_FALLBACK_MS_BROWSER = 800;

export default function GetMailButton() {
  const { t } = useTranslation("nav");
  const { data: accounts } = useAccounts();
  const triggerSync = useTriggerSync();
  const [spinning, setSpinning] = useState(false);
  const timeoutRef = useRef<number | undefined>(undefined);

  const stop = useCallback(() => {
    setSpinning(false);
    if (timeoutRef.current !== undefined) {
      window.clearTimeout(timeoutRef.current);
      timeoutRef.current = undefined;
    }
  }, []);

  // A real sync ends with one of these events; stop the pinwheel when they land.
  useEvent("sync:complete", stop);
  useEvent("sync:error", stop);

  // Clear the fallback timer if the rail unmounts mid-spin.
  useEffect(
    () => () => {
      if (timeoutRef.current !== undefined) window.clearTimeout(timeoutRef.current);
    },
    [],
  );

  const active = (accounts ?? []).filter((a) => a.isActive);

  const onClick = useCallback(() => {
    const list = (accounts ?? []).filter((a) => a.isActive);
    if (list.length === 0) return;
    setSpinning(true);
    for (const account of list) triggerSync.mutate(account.id);
    if (timeoutRef.current !== undefined) window.clearTimeout(timeoutRef.current);
    timeoutRef.current = window.setTimeout(
      () => setSpinning(false),
      isTauri() ? SPIN_FALLBACK_MS_TAURI : SPIN_FALLBACK_MS_BROWSER,
    );
  }, [accounts, triggerSync]);

  return (
    <button
      type="button"
      onClick={onClick}
      disabled={active.length === 0}
      title={t("nav_get_mail")}
      aria-label={t("nav_get_mail")}
      className={cn(
        "flex h-5 w-5 shrink-0 items-center justify-center rounded-chip text-p7",
        "transition-colors hover:text-p10 disabled:cursor-default disabled:opacity-40",
      )}
    >
      {/* Four-blade pinwheel; alternating shades read as motion, and `animate-spin`
          turns it while a fetch is in flight. */}
      <svg
        width="14"
        height="14"
        viewBox="0 0 16 16"
        fill="none"
        aria-hidden
        className={cn(spinning && "animate-spin")}
      >
        <path d="M8 1 A7 7 0 0 1 15 8 L8 8 Z" fill="currentColor" opacity="0.9" />
        <path d="M15 8 A7 7 0 0 1 8 15 L8 8 Z" fill="currentColor" opacity="0.5" />
        <path d="M8 15 A7 7 0 0 1 1 8 L8 8 Z" fill="currentColor" opacity="0.9" />
        <path d="M1 8 A7 7 0 0 1 8 1 L8 8 Z" fill="currentColor" opacity="0.5" />
        <circle cx="8" cy="8" r="1.4" fill="var(--p3)" />
      </svg>
    </button>
  );
}
