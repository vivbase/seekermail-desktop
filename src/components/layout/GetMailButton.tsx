// Icon-only "Get Mail" control in the nav rail (07 §3). A pinwheel that spins
// while a manual sync runs — no permanent text. A light label, centered in the
// nav rail, reads "Receive Emails" on hover and "Retrieving Email…" while a fetch
// is in flight. Clicking force-syncs every active account via `trigger_sync` (the
// same path IMAP IDLE pokes automatically). The spin is non-blocking: it animates
// the icon in place and never covers the UI. It stops on the sync:complete /
// sync:error event, or a fallback timeout off-Tauri where no events fire.
//
// The label is rendered through a portal to <body> and fixed-positioned at the
// rail's horizontal center: the nav rail is an `overflow-y-auto` column, so a
// label sitting above its first row would otherwise be clipped at the rail's top.
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
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
  const [hovered, setHovered] = useState(false);
  const [anchor, setAnchor] = useState<{ left: number; top: number } | null>(null);
  const btnRef = useRef<HTMLButtonElement>(null);
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

  // The floating label shows while hovering/focused or while a fetch runs.
  const labelVisible = spinning || hovered;

  // Center the label horizontally on the nav rail (`.sidebar`) and float it just
  // above the button row. The portal needs absolute viewport coordinates, so we
  // read the rail's center and the button's top here (falls back to the button's
  // own center if the rail isn't found).
  useLayoutEffect(() => {
    if (!labelVisible) return;
    const el = btnRef.current;
    if (!el) return;
    const btn = el.getBoundingClientRect();
    const rail = el.closest<HTMLElement>(".sidebar")?.getBoundingClientRect();
    setAnchor({
      left: rail ? rail.left + rail.width / 2 : btn.left + btn.width / 2,
      top: btn.top,
    });
  }, [labelVisible]);

  return (
    <>
      <button
        ref={btnRef}
        type="button"
        onClick={onClick}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
        onFocus={() => setHovered(true)}
        onBlur={() => setHovered(false)}
        disabled={active.length === 0}
        aria-label={t("nav_get_mail")}
        className={cn(
          "flex h-5 w-5 shrink-0 items-center justify-center rounded-chip text-p7",
          "transition-colors hover:text-p10 disabled:cursor-default disabled:opacity-40",
        )}
      >
        {/* The wrapper carries the grow (scale) so it composes with the SVG's spin
            (rotate) — both touch `transform`, so they must live on separate nodes. */}
        <span
          className={cn(
            "inline-flex transition-transform duration-200 ease-out",
            spinning && "scale-125",
          )}
        >
          {/* Four-blade pinwheel; alternating shades read as motion, and
              `animate-spin-fast` whirls it briskly while a fetch is in flight. */}
          <svg
            width="14"
            height="14"
            viewBox="0 0 16 16"
            fill="none"
            aria-hidden
            className={cn(spinning && "animate-spin-fast")}
          >
            <path d="M8 1 A7 7 0 0 1 15 8 L8 8 Z" fill="currentColor" opacity="0.9" />
            <path d="M15 8 A7 7 0 0 1 8 15 L8 8 Z" fill="currentColor" opacity="0.5" />
            <path d="M8 15 A7 7 0 0 1 1 8 L8 8 Z" fill="currentColor" opacity="0.9" />
            <path d="M1 8 A7 7 0 0 1 8 1 L8 8 Z" fill="currentColor" opacity="0.5" />
            <circle cx="8" cy="8" r="1.4" fill="var(--p3)" />
          </svg>
        </span>
      </button>

      {labelVisible &&
        anchor &&
        createPortal(
          <span
            aria-hidden
            style={{
              position: "fixed",
              left: anchor.left,
              top: anchor.top,
              transform: "translate(-50%, calc(-100% - 5px))",
            }}
            className="pointer-events-none z-50 whitespace-nowrap font-ui text-[10px] tracking-[0.04em] text-p7"
          >
            {spinning ? t("nav_retrieving_mail") : t("nav_get_mail")}
          </span>,
          document.body,
        )}
    </>
  );
}
