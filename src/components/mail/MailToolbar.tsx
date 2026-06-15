// Bottom action toolbar for the L2 reading view (T041, F_G3 §4.5).
// Reply / Reply all / Forward seed the compose store then navigate to /compose.
// Archive / Delete call mutations; onArchived / onDeleted callbacks let the
// parent (the route) show the UndoToast. Mark unread + Star are in the More menu.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import type { MailDetail } from "@shared/bindings";

import {
  useArchiveMail,
  useDeleteMail,
  useSetMailRead,
  useSetMailStarred,
} from "@/ipc/queries/mail";
import { useCompose } from "@/stores/compose";
import { cn } from "@/lib/cn";
import { AiReplyButton } from "./AiReplyButton";

interface MailToolbarProps {
  mail: MailDetail;
  /**
   * Optionally controlled from outside (e.g. by useL2Shortcuts `.` key).
   * When omitted, the More menu manages its own open state internally.
   */
  moreOpen?: boolean;
  onSetMoreOpen?: (open: boolean) => void;
  /** Called when archive succeeds — parent is responsible for UndoToast. */
  onArchived?: (id: string) => void;
  /** Called when delete succeeds — parent is responsible for UndoToast. */
  onDeleted?: (id: string) => void;
}

// ── Toolbar button ─────────────────────────────────────────────────────────────

interface ToolbarButtonProps {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  destructive?: boolean;
  icon: React.ReactNode;
}

function ToolbarButton({ label, onClick, disabled, destructive, icon }: ToolbarButtonProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={label}
      aria-label={label}
      className={cn(
        "flex items-center gap-1.5 rounded-chip px-3 py-1.5 font-ui text-xs uppercase tracking-wider transition-colors",
        "focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
        destructive
          ? "hover:bg-red/10 text-red disabled:opacity-50"
          : "text-p8 hover:bg-p4 hover:text-p10 disabled:opacity-50",
      )}
    >
      {icon}
      <span className="hidden sm:inline">{label}</span>
    </button>
  );
}

// ── Inline SVG icons (no CDN) ──────────────────────────────────────────────────

const Icons = {
  Reply: () => (
    <svg
      width="14"
      height="14"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <path strokeLinecap="round" strokeLinejoin="round" d="M6 4 2 8l4 4M2 8h8a4 4 0 0 1 4 4v0" />
    </svg>
  ),
  ReplyAll: () => (
    <svg
      width="14"
      height="14"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        d="M4 4 0 8l4 4M8 4 4 8l4 4M4 8h8a4 4 0 0 1 4 4v0"
      />
    </svg>
  ),
  Forward: () => (
    <svg
      width="14"
      height="14"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <path strokeLinecap="round" strokeLinejoin="round" d="M10 4l4 4-4 4M14 8H6a4 4 0 0 0-4 4v0" />
    </svg>
  ),
  Archive: () => (
    <svg
      width="14"
      height="14"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        d="M14 5H2v9h12V5ZM14 2H2v3h12V2ZM6.5 8h3"
      />
    </svg>
  ),
  Delete: () => (
    <svg
      width="14"
      height="14"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        d="M2 4h12M5 4V2h6v2M6 7v5M10 7v5M3 4l1 10h8l1-10"
      />
    </svg>
  ),
  MarkUnread: () => (
    <svg
      width="14"
      height="14"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        d="M1 4a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V4Z"
      />
      <path strokeLinecap="round" d="M1 4l7 5 7-5" />
    </svg>
  ),
  Star: (props: { filled: boolean }) => (
    <svg
      width="14"
      height="14"
      viewBox="0 0 16 16"
      fill={props.filled ? "currentColor" : "none"}
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      <path
        strokeLinejoin="round"
        d="M8 1.5 10 6h4.5l-3.5 2.5 1.3 4.5L8 10.5l-4.3 2.5 1.3-4.5L1.5 6H6Z"
      />
    </svg>
  ),
} as const;

// ── MailToolbar ────────────────────────────────────────────────────────────────

export function MailToolbar({
  mail,
  moreOpen: moreOpenProp,
  onSetMoreOpen,
  onArchived,
  onDeleted,
}: MailToolbarProps) {
  const { t } = useTranslation("reading");
  const navigate = useNavigate();

  const archiveMail = useArchiveMail();
  const deleteMail = useDeleteMail();
  const setMailRead = useSetMailRead();
  const setMailStarred = useSetMailStarred();
  const openCompose = useCompose((s) => s.open);

  const [moreOpenInternal, setMoreOpenInternal] = useState(false);

  // Support optional controlled mode from a parent (e.g. keyboard shortcut).
  const moreOpen = moreOpenProp !== undefined ? moreOpenProp : moreOpenInternal;
  const setMoreOpen: (open: boolean) => void =
    onSetMoreOpen !== undefined ? onSetMoreOpen : setMoreOpenInternal;

  // ── Compose navigation ────────────────────────────────────────────────────

  const navigateReply = () => {
    openCompose({
      inReplyTo: mail.id,
      subject: mail.subject.startsWith("Re: ") ? mail.subject : `Re: ${mail.subject}`,
      to: mail.fromEmail,
    });
    navigate("/compose");
  };

  const navigateReplyAll = () => {
    const toAddrs = [mail.fromEmail, ...mail.to.map((r) => r.email)].join(", ");
    openCompose({
      inReplyTo: mail.id,
      subject: mail.subject.startsWith("Re: ") ? mail.subject : `Re: ${mail.subject}`,
      to: toAddrs,
      cc: mail.cc.map((r) => r.email).join(", "),
    });
    navigate("/compose");
  };

  const navigateForward = () => {
    openCompose({
      subject: mail.subject.startsWith("Fwd: ") ? mail.subject : `Fwd: ${mail.subject}`,
    });
    navigate("/compose");
  };

  // ── Destructive actions ───────────────────────────────────────────────────

  const handleArchive = () => {
    archiveMail.mutate(mail.id, {
      onSuccess: () => {
        onArchived?.(mail.id);
        navigate("/");
      },
    });
  };

  const handleDelete = () => {
    deleteMail.mutate(mail.id, {
      onSuccess: () => {
        onDeleted?.(mail.id);
        navigate("/");
      },
    });
  };

  const handleMarkUnread = () => {
    setMailRead.mutate({ mailId: mail.id, isRead: false });
    navigate("/");
  };

  const handleToggleStar = () => {
    setMailStarred.mutate({ mailId: mail.id, isStarred: !mail.isStarred });
  };

  return (
    <footer className="border-t border-divider bg-surface">
      <div className="mx-auto flex max-w-[680px] flex-wrap items-center justify-between gap-2 px-5 py-2">
        {/* Primary compose actions */}
        <div className="flex items-center gap-1">
          <ToolbarButton label={t("reply")} onClick={navigateReply} icon={<Icons.Reply />} />
          <ToolbarButton
            label={t("reply_all")}
            onClick={navigateReplyAll}
            icon={<Icons.ReplyAll />}
          />
          <ToolbarButton label={t("forward")} onClick={navigateForward} icon={<Icons.Forward />} />
          {/* E1 manual AI reply (T078) — one of the two sanctioned entry points. */}
          <AiReplyButton mail={mail} />
        </div>

        {/* Management actions */}
        <div className="flex items-center gap-1">
          <ToolbarButton
            label={t("archive")}
            onClick={handleArchive}
            disabled={archiveMail.isPending}
            icon={<Icons.Archive />}
          />
          <ToolbarButton
            label={t("delete")}
            onClick={handleDelete}
            disabled={deleteMail.isPending}
            destructive
            icon={<Icons.Delete />}
          />

          {/* More menu */}
          <div className="relative">
            <button
              type="button"
              onClick={() => setMoreOpen(!moreOpen)}
              aria-expanded={moreOpen}
              aria-haspopup="menu"
              aria-label="More actions"
              className="flex items-center rounded-chip px-2 py-1.5 font-ui text-xs text-p8 transition-colors hover:bg-p4 hover:text-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
            >
              <svg
                width="14"
                height="14"
                viewBox="0 0 16 16"
                fill="currentColor"
                aria-hidden="true"
              >
                <circle cx="4" cy="8" r="1.25" />
                <circle cx="8" cy="8" r="1.25" />
                <circle cx="12" cy="8" r="1.25" />
              </svg>
            </button>

            {moreOpen && (
              <div
                role="menu"
                className="absolute bottom-full right-0 mb-1 min-w-[160px] rounded-card border border-divider bg-surface py-1 shadow-card"
              >
                <button
                  role="menuitem"
                  type="button"
                  onClick={() => {
                    handleMarkUnread();
                    setMoreOpen(false);
                  }}
                  className="flex w-full items-center gap-2 px-3 py-2 font-ui text-xs text-p8 hover:bg-p4 hover:text-p10 focus:outline-none focus-visible:bg-p4"
                >
                  <Icons.MarkUnread />
                  {t("mark_unread")}
                </button>
                <button
                  role="menuitem"
                  type="button"
                  onClick={() => {
                    handleToggleStar();
                    setMoreOpen(false);
                  }}
                  className="flex w-full items-center gap-2 px-3 py-2 font-ui text-xs text-p8 hover:bg-p4 hover:text-p10 focus:outline-none focus-visible:bg-p4"
                >
                  <Icons.Star filled={mail.isStarred} />
                  {mail.isStarred ? t("unstar") : t("star")}
                </button>
              </div>
            )}
          </div>
        </div>
      </div>
    </footer>
  );
}
