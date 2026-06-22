// Remote-image control bar (T029, hardened). Remote images are blocked by
// default; this bar is the prominent, one-click affordance to load them. The
// load NEVER lets the webview hit the origin — every image is fetched through
// the backend (no cookies / Referer / User-Agent) and swapped in as a `data:`
// URI (revealRemoteImages). "This message" loads once; "always" persists the
// sender so future mail auto-loads. Allow-listed senders auto-reveal silently.
import { useEffect, useRef, useState, type RefObject } from "react";
import { useTranslation } from "react-i18next";

import { useAllowRemoteImages, useFetchRemoteImage } from "@/ipc/queries/mail";
import { revealRemoteImages } from "@/lib/mailImages";

interface RemoteImageBarProps {
  mailId: string;
  senderEmail: string;
  imagesAllowed: boolean;
  bodyRef: RefObject<HTMLDivElement>;
}

export default function RemoteImageBar({
  mailId,
  senderEmail,
  imagesAllowed,
  bodyRef,
}: RemoteImageBarProps) {
  const { t } = useTranslation();
  const allow = useAllowRemoteImages();
  const fetchRemote = useFetchRemoteImage();
  const [phase, setPhase] = useState<"idle" | "loading" | "loaded">("idle");
  const autoRan = useRef(false);

  const reveal = async () => {
    setPhase("loading");
    await revealRemoteImages(bodyRef.current, (url) => fetchRemote.mutateAsync(url));
    setPhase("loaded");
  };

  // Allow-listed sender: load automatically (once), no bar shown.
  useEffect(() => {
    if (imagesAllowed && !autoRan.current) {
      autoRan.current = true;
      void reveal();
    }
    // reveal closes over stable refs/mutations; only re-run on allow change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [imagesAllowed]);

  // Allow-listed senders never show the bar (images load automatically above).
  if (imagesAllowed) return null;

  // After an explicit in-session load, replace the bar with a small confirmation.
  if (phase === "loaded") {
    return (
      <p className="mb-3 font-ui text-[10px] uppercase tracking-wider text-p8">
        {t("images_loaded")}
      </p>
    );
  }

  const loading = phase === "loading";

  const loadThisMessage = () => {
    void reveal();
    // Audit only — the reveal itself is the frontend DOM swap (T029 §3).
    allow.mutate({ mailId, scope: { type: "thisMessage" } });
  };

  const alwaysThisSender = () => {
    void reveal();
    allow.mutate({ mailId, scope: { type: "alwaysSender", senderEmail } });
  };

  return (
    <div className="bg-amber/10 mb-4 flex flex-wrap items-center gap-3 rounded-card border-l-4 border-amber px-4 py-3">
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={1.8}
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
        className="h-5 w-5 shrink-0 text-amber"
      >
        <rect x="3" y="4" width="18" height="16" rx="2" />
        <circle cx="8.5" cy="9.5" r="1.5" />
        <path d="M21 15l-5-5L5 20" />
        <line x1="3" y1="3" x2="21" y2="21" />
      </svg>

      <div className="min-w-0 flex-1">
        <p className="font-ui text-[11px] font-semibold uppercase tracking-wider text-p9">
          {t("images_blocked_title")}
        </p>
        <p className="mt-0.5 font-body text-xs leading-snug text-p8">
          {t("images_blocked_notice")}
        </p>
      </div>

      <div className="flex shrink-0 items-center gap-2">
        <button
          type="button"
          onClick={loadThisMessage}
          disabled={loading}
          className="rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white hover:bg-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9 disabled:opacity-50"
        >
          {loading ? t("images_loading") : t("images_load_this")}
        </button>
        <button
          type="button"
          onClick={alwaysThisSender}
          disabled={loading}
          className="rounded-chip border border-p5 px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p8 hover:bg-p3 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9 disabled:opacity-50"
        >
          {t("images_load_always")}
        </button>
      </div>
    </div>
  );
}
