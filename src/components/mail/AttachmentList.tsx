// Attachment list for the L2 reading view (T042). Renders non-inline attachments
// only (isInline=true are excluded — they appear via cid: in the body).
// Per-attachment row: icon by MIME type, filename, human size, Download / Open /
// Show-in-folder buttons. Download progress subscribes to the attachment_progress
// query key set by the global event handler (registerIpcEvents in events.ts).
import { useEffect, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import type { Attachment } from "@shared/bindings";

import {
  useAttachmentsForMail,
  useDownloadAttachment,
  useOpenAttachment,
  useRevealAttachment,
} from "@/ipc/queries/attachments";
import type { AttachmentProgressPayload } from "@shared/bindings";
import { formatBytes } from "@/lib/formatBytes";
import { cn } from "@/lib/cn";

interface AttachmentListProps {
  mailId: string;
}

// ── MIME type → icon mapping ──────────────────────────────────────────────────

type MimeCategory = "pdf" | "image" | "archive" | "doc" | "other";

function mimeCategory(contentType: string): MimeCategory {
  if (contentType === "application/pdf") return "pdf";
  if (contentType.startsWith("image/")) return "image";
  if (
    contentType === "application/zip" ||
    contentType === "application/gzip" ||
    contentType === "application/x-tar" ||
    contentType === "application/x-7z-compressed" ||
    contentType === "application/x-rar-compressed"
  )
    return "archive";
  if (
    contentType.startsWith("application/msword") ||
    contentType.startsWith("application/vnd.openxmlformats") ||
    contentType.startsWith("application/vnd.ms-") ||
    contentType.startsWith("text/")
  )
    return "doc";
  return "other";
}

// Inline SVG icons — no external CDN.
function MimeIcon({ category }: { category: MimeCategory }) {
  const base = "h-8 w-8 shrink-0";
  switch (category) {
    case "pdf":
      return (
        <svg
          className={cn(base, "text-terra")}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          aria-hidden="true"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            d="M9 12h6M9 16h6M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z"
          />
          <path strokeLinecap="round" strokeLinejoin="round" d="M13 2v7h7" />
        </svg>
      );
    case "image":
      return (
        <svg
          className={cn(base, "text-sage")}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          aria-hidden="true"
        >
          <rect x="3" y="3" width="18" height="18" rx="2" />
          <circle cx="8.5" cy="8.5" r="1.5" />
          <path strokeLinecap="round" strokeLinejoin="round" d="m21 15-5-5L5 21" />
        </svg>
      );
    case "archive":
      return (
        <svg
          className={cn(base, "text-amber")}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          aria-hidden="true"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            d="M21 8v13H3V8M23 3H1v5h22V3zM10 12h4"
          />
        </svg>
      );
    case "doc":
      return (
        <svg
          className={cn(base, "text-slate")}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          aria-hidden="true"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"
          />
          <path strokeLinecap="round" strokeLinejoin="round" d="M14 2v6h6M16 13H8M16 17H8M10 9H8" />
        </svg>
      );
    default:
      return (
        <svg
          className={cn(base, "text-p7")}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          aria-hidden="true"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z"
          />
          <path strokeLinecap="round" strokeLinejoin="round" d="M13 2v7h7" />
        </svg>
      );
  }
}

// ── Single attachment row ─────────────────────────────────────────────────────

interface AttachmentItemProps {
  attachment: Attachment;
  /** When true, briefly highlight + scroll this row into view (T110 deep link). */
  highlight?: boolean;
}

function AttachmentItem({ attachment, highlight = false }: AttachmentItemProps) {
  const { t } = useTranslation("reading");
  const qc = useQueryClient();
  const rowRef = useRef<HTMLDivElement>(null);
  const [highlighted, setHighlighted] = useState(false);

  // T110: when arriving from an attachment search hit, flash a 2px amber ring on
  // the targeted row for 1.5s and scroll it into view. Timer cleared on unmount.
  useEffect(() => {
    if (!highlight) return;
    setHighlighted(true);
    rowRef.current?.scrollIntoView({ behavior: "smooth", block: "center" });
    const timer = setTimeout(() => setHighlighted(false), 1500);
    return () => clearTimeout(timer);
  }, [highlight]);
  const downloadMutation = useDownloadAttachment();
  const openMutation = useOpenAttachment();
  const revealMutation = useRevealAttachment();

  // Progress is stored in TQ cache by the global attachment:progress handler.
  // registerIpcEvents (events.ts) writes: qc.setQueryData(["attachment_progress", id], payload)
  // where payload is AttachmentProgressPayload { attachmentId, pct }.
  const progressData = qc.getQueryData<AttachmentProgressPayload>([
    "attachment_progress",
    attachment.id,
  ]);
  const pct = progressData?.pct ?? 0;
  const isDownloading = downloadMutation.isPending;

  const isDownloaded = attachment.downloaded;
  const category = mimeCategory(attachment.contentType);

  const handleDownload = () => {
    downloadMutation.mutate(attachment.id);
  };

  const handleOpen = () => {
    openMutation.mutate(attachment.id);
  };

  const handleReveal = () => {
    revealMutation.mutate(attachment.id);
  };

  return (
    <div
      ref={rowRef}
      className={cn(
        "flex items-center gap-3 rounded-card border bg-surface p-3 shadow-card transition-[box-shadow,border-color] duration-500",
        highlighted ? "border-amber ring-2 ring-amber" : "border-divider",
      )}
    >
      <MimeIcon category={category} />

      <div className="min-w-0 flex-1">
        <p className="truncate font-ui text-sm font-medium text-p9" title={attachment.filename}>
          {attachment.filename}
        </p>
        <p className="font-mono text-xs text-p7">{formatBytes(attachment.sizeBytes)}</p>

        {/* Download progress bar */}
        {isDownloading && (
          <>
            <progress
              role="progressbar"
              aria-valuenow={pct}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-label={t("lbl_downloading", { filename: attachment.filename })}
              value={pct}
              max={100}
              className="mt-1 h-1 w-full appearance-none rounded-full [&::-webkit-progress-bar]:rounded-full [&::-webkit-progress-bar]:bg-p4 [&::-webkit-progress-value]:rounded-full [&::-webkit-progress-value]:bg-p9"
            />
            <p className="mt-0.5 font-ui text-[10px] text-p7">{t("attachment_downloading")}</p>
          </>
        )}

        {/* a11y live region: announces download completion */}
        <span aria-live="polite" aria-atomic="true" className="sr-only">
          {isDownloaded ? t("msg_download_complete", { name: attachment.filename }) : ""}
        </span>
      </div>

      {/* Actions */}
      <div className="flex shrink-0 items-center gap-1">
        {!isDownloaded && !isDownloading && (
          <button
            type="button"
            onClick={handleDownload}
            aria-label={t("btn_download_file", { filename: attachment.filename })}
            className="rounded-chip bg-p9 px-2.5 py-1 font-ui text-[10px] uppercase tracking-wider text-white transition-colors hover:bg-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
          >
            {t("attachment_download")}
          </button>
        )}

        {isDownloaded && (
          <>
            <button
              type="button"
              onClick={handleOpen}
              disabled={openMutation.isPending}
              aria-label={t("btn_open_file", { filename: attachment.filename })}
              className="rounded-chip bg-p9 px-2.5 py-1 font-ui text-[10px] uppercase tracking-wider text-white transition-colors hover:bg-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9 disabled:opacity-50"
            >
              {t("attachment_open")}
            </button>
            <button
              type="button"
              onClick={handleReveal}
              disabled={revealMutation.isPending}
              aria-label={t("attachment_reveal")}
              title={t("attachment_reveal")}
              className="rounded-chip px-2 py-1 font-ui text-[10px] text-p7 transition-colors hover:bg-p4 hover:text-p9 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9 disabled:opacity-50"
            >
              {/* Folder icon */}
              <svg
                width="12"
                height="12"
                viewBox="0 0 16 16"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.5"
                aria-hidden="true"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  d="M2 5a2 2 0 0 1 2-2h2.5l2 2H12a2 2 0 0 1 2 2v5a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5z"
                />
              </svg>
            </button>
          </>
        )}
      </div>
    </div>
  );
}

// ── AttachmentList ─────────────────────────────────────────────────────────────

export function AttachmentList({ mailId }: AttachmentListProps) {
  const { t } = useTranslation("reading");
  const { data: attachments, isLoading } = useAttachmentsForMail(mailId);
  const [searchParams] = useSearchParams();
  const highlightId = searchParams.get("attachmentId");

  // Filter out inline attachments (rendered via cid: in the body).
  const visible = (attachments ?? []).filter((a) => !a.isInline);

  if (isLoading || visible.length === 0) return null;

  return (
    <section aria-label={t("attachments_title")} className="mt-4 border-t border-divider pt-4">
      <p className="section-label mb-3">{t("attachments_title")}</p>
      <ul role="list" className="space-y-2">
        {visible.map((attachment) => (
          <li key={attachment.id}>
            <AttachmentItem attachment={attachment} highlight={attachment.id === highlightId} />
          </li>
        ))}
      </ul>
    </section>
  );
}
