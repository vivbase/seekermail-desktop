// L2 immersive reading view (T041). Route: /mail/:id
// Fetches full MailDetail, marks the mail read on mount. The top nav bar and
// MailHeader are anchored left near the nav rail (max-w 720), while MailBody →
// AttachmentList sit in a centred 680 px reading column. A sticky MailToolbar
// pins to the bottom and a collapsible ThreadDrawer sits on the right.
// Keyboard shortcuts are active via useL2Shortcuts.
// Scroll position is persisted to the selection store on unmount.
import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { useTranslation } from "react-i18next";

import {
  useInlineImages,
  useMailDetail,
  useSetMailRead,
  useSetMailStarred,
  useTrackerInfo,
} from "@/ipc/queries/mail";
import { useRiskEvents } from "@/ipc/queries/risk";
import { T4_RISK_LEVEL } from "@/ipc/legal";
import { useSelection } from "@/stores/selection";
import { useAccounts } from "@/ipc/queries/accounts";
import { type AccountColorToken } from "@/lib/accountColor";

import { MailHeader } from "@/components/mail/MailHeader";
import { MailBody } from "@/components/mail/MailBody";
import { MailToolbar } from "@/components/mail/MailToolbar";
import { ReadingSizeControl } from "@/components/mail/ReadingSizeControl";
import { RiskAlertBanner } from "@/components/mail/RiskAlertBanner";
import { ThreadDrawer } from "@/components/mail/ThreadDrawer";
import { AttachmentList } from "@/components/mail/AttachmentList";
import { useL2Shortcuts } from "@/hooks/useL2Shortcuts";
import { useUndoToast } from "@/components/ui/UndoToast";

// ── Loading skeleton ──────────────────────────────────────────────────────────

function LoadingSkeleton() {
  const { t } = useTranslation("reading");
  return (
    <div className="flex h-full items-center justify-center bg-surface">
      <p className="font-body text-sm text-p7">{t("loading")}</p>
    </div>
  );
}

// ── Not-found state ───────────────────────────────────────────────────────────

function NotFound() {
  const { t } = useTranslation("reading");
  const navigate = useNavigate();
  return (
    <div className="flex h-full flex-col items-center justify-center gap-4 bg-surface">
      <p className="font-body text-p7">{t("not_found")}</p>
      <button
        type="button"
        onClick={() => navigate("/")}
        className="rounded-chip bg-p9 px-4 py-2 font-ui text-xs uppercase tracking-wider text-white hover:bg-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
      >
        {t("back")}
      </button>
    </div>
  );
}

// ── Main route ────────────────────────────────────────────────────────────────

export default function MailDetail() {
  const { id: mailId } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const { t } = useTranslation("reading");

  // ── Data fetching ─────────────────────────────────────────────────────────

  const { data: mail, isLoading, isError } = useMailDetail(mailId);
  const { data: trackerInfo } = useTrackerInfo(mailId ?? "");
  // Inline (cid:) images: only fetched when the body actually references them.
  const hasInlineImages = !!mail?.bodyHtml && /src=["']cid:/i.test(mail.bodyHtml);
  const { data: inlineImages } = useInlineImages(mailId ?? "", hasInlineImages);
  const { data: accounts } = useAccounts();
  // Open risk events for this mail (T071 §3.2). The `risk:alert` push event
  // invalidates ['riskEvents'] (events.ts), so T4 banners appear live.
  const { data: riskEvents } = useRiskEvents({
    mailId: mailId ?? "",
    status: "open",
  });

  // ── Store hooks ───────────────────────────────────────────────────────────

  const selectMail = useSelection((s) => s.selectMail);
  const mailScrollPositions = useSelection((s) => s.mailScrollPositions);
  const setScrollPosition = useSelection((s) => s.setScrollPosition);

  // ── Mutations ─────────────────────────────────────────────────────────────

  const setMailRead = useSetMailRead();
  const setMailStarred = useSetMailStarred();

  // ── Refs ──────────────────────────────────────────────────────────────────

  /** The scrollable outer container — used for scroll-position persistence. */
  const scrollRef = useRef<HTMLDivElement>(null);
  /** Wrapper div focused on mount for screen-reader route-change announcement. */
  const h1Ref = useRef<HTMLDivElement>(null);

  // ── UI state ──────────────────────────────────────────────────────────────

  const [drawerOpen, setDrawerOpen] = useState(false);
  const { toastEl: undoToastEl, showUndoToast } = useUndoToast();
  const [moreMenuOpen, setMoreMenuOpen] = useState(false);

  // ── Effects ───────────────────────────────────────────────────────────────

  // Update selection store when the mail ID changes.
  useEffect(() => {
    selectMail(mailId ?? null);
    return () => selectMail(null);
  }, [mailId, selectMail]);

  // Mark as read on mount (optimistic update is handled inside the mutation).
  useEffect(() => {
    if (mail && !mail.isRead && mailId) {
      setMailRead.mutate({ mailId, isRead: true });
    }
    // Deliberately omit setMailRead from deps — we only want to fire once per mailId.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mailId, mail?.isRead]);

  // Restore scroll position on mount; persist on unmount.
  useEffect(() => {
    const pos = mailId ? (mailScrollPositions.get(mailId) ?? 0) : 0;
    scrollRef.current?.scrollTo({ top: pos });

    return () => {
      if (mailId && scrollRef.current) {
        setScrollPosition(mailId, scrollRef.current.scrollTop);
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mailId]);

  // Focus the h1 after render for screen-reader route-change announcement (dev/11 §3).
  useEffect(() => {
    if (mail) {
      // Small delay to ensure the DOM is painted.
      const raf = requestAnimationFrame(() => {
        h1Ref.current?.focus();
      });
      return () => cancelAnimationFrame(raf);
    }
  }, [mail]);

  // ── Derived values ────────────────────────────────────────────────────────

  const account = accounts?.find((a) => a.id === mail?.accountId);
  const colorToken: AccountColorToken =
    (account?.colorToken as AccountColorToken | undefined) ?? "slate";

  const isStarred = mail?.isStarred ?? false;

  // T4 alerts stack — never folded together, never dismissable (T071 §3.2).
  const t4Events = (riskEvents ?? []).filter((e) => e.riskLevel === T4_RISK_LEVEL);

  const handleToggleStar = useCallback(() => {
    if (!mail) return;
    setMailStarred.mutate({ mailId: mail.id, isStarred: !isStarred });
  }, [mail, isStarred, setMailStarred]);

  // ── Keyboard shortcuts ────────────────────────────────────────────────────

  useL2Shortcuts({
    mail,
    onToggleMore: () => setMoreMenuOpen((v) => !v),
    // After archive/delete the shortcut handler already navigates to "/".
    // Show UndoToast so the user can jump back to the mail if they change their mind.
    onArchived: (id) => showUndoToast(t("archived_toast"), () => navigate(`/mail/${id}`)),
    onDeleted: (id) => showUndoToast(t("deleted_toast"), () => navigate(`/mail/${id}`)),
  });

  // ── Render ────────────────────────────────────────────────────────────────

  if (isLoading) return <LoadingSkeleton />;
  if (isError || !mail) return <NotFound />;

  return (
    <div className="flex h-full w-full overflow-hidden bg-surface">
      {/* Main reading area: flex-col so the toolbar sticks to the bottom */}
      <div className="flex min-h-0 flex-1 flex-col">
        {/* Scrollable content */}
        <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto">
          {/* Top navigation bar */}
          <div className="sticky top-0 z-10 border-b border-divider bg-surface backdrop-blur-sm">
            <div className="flex max-w-[720px] items-center gap-3 px-8 py-2">
              <button
                type="button"
                onClick={() => navigate(-1)}
                aria-label={t("back")}
                className="flex items-center gap-1.5 rounded-chip px-2 py-1.5 font-ui text-xs text-p8 transition-colors hover:bg-p4 hover:text-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
              >
                <svg
                  width="14"
                  height="14"
                  viewBox="0 0 16 16"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.5"
                  aria-hidden="true"
                >
                  <path strokeLinecap="round" strokeLinejoin="round" d="M10 12 6 8l4-4" />
                </svg>
                {t("back")}
              </button>

              {/* Reading text-size stepper — scales the email body only (analysis 25) */}
              <div className="ms-auto">
                <ReadingSizeControl />
              </div>

              {/* Thread toggle */}
              <button
                type="button"
                onClick={() => setDrawerOpen((v) => !v)}
                aria-pressed={drawerOpen}
                aria-label={drawerOpen ? t("hide_thread") : t("show_thread")}
                className="flex items-center gap-1.5 rounded-chip px-2 py-1.5 font-ui text-xs text-p8 transition-colors hover:bg-p4 hover:text-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
              >
                <svg
                  width="14"
                  height="14"
                  viewBox="0 0 16 16"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.5"
                  aria-hidden="true"
                >
                  <path strokeLinecap="round" strokeLinejoin="round" d="M2 4h12M2 8h8M2 12h10" />
                </svg>
                {drawerOpen ? t("hide_thread") : t("show_thread")}
              </button>
            </div>
          </div>

          {/* Header band — anchored left near the nav rail (T4 banners + header).
              The body below keeps its own centred reading column. */}
          <div className="w-full max-w-[720px] px-8 pt-6">
            {/* T4 non-dismissable risk banners — above MailHeader (T071 §3.2). */}
            {t4Events.length > 0 && (
              <div className="mb-5 flex flex-col gap-3">
                {t4Events.map((event) => (
                  <RiskAlertBanner key={event.id} event={event} />
                ))}
              </div>
            )}

            {/* Focus wrapper — receives focus on route mount (screen-reader announce). */}
            <div tabIndex={-1} ref={h1Ref} className="focus:outline-none">
              <MailHeader
                mail={mail}
                colorToken={colorToken}
                isStarred={isStarred}
                onToggleStar={handleToggleStar}
              />
            </div>
          </div>

          {/* Body reading column — stays centred for a comfortable line length */}
          <div className="mx-auto w-full max-w-[680px] px-5 pb-6">
            <MailBody mail={mail} trackerInfo={trackerInfo} inlineImages={inlineImages} />

            {mail.hasAttachments && <AttachmentList mailId={mail.id} />}
          </div>
        </div>

        {/* Bottom toolbar — stays pinned; moreOpen shared with keyboard shortcut */}
        <MailToolbar
          mail={mail}
          moreOpen={moreMenuOpen}
          onSetMoreOpen={setMoreMenuOpen}
          onArchived={(id) => showUndoToast(t("archived_toast"), () => navigate(`/mail/${id}`))}
          onDeleted={(id) => showUndoToast(t("deleted_toast"), () => navigate(`/mail/${id}`))}
        />
      </div>

      {/* Thread drawer */}
      <ThreadDrawer currentMail={mail} isOpen={drawerOpen} onClose={() => setDrawerOpen(false)} />

      {/* Undo toast portal */}
      <div className="pointer-events-none fixed inset-x-4 bottom-20 z-50 flex justify-end">
        <div className="pointer-events-auto">{undoToastEl}</div>
      </div>
    </div>
  );
}
