// Attachment staging bar (T044, F_G4 §4.5). Lets the user stage local files
// before sending; real upload/encoding is out of scope for v0.x. Staged filenames
// are held in local component state — not the compose store — because the backend
// currently accepts attachments as part of send_mail (future card). The user sees
// honest chips they can remove; no base64 conversion happens here yet.

import { useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/cn";

// ── Types ────────────────────────────────────────────────────────────────────

interface StagedFile {
  /** Stable internal key. */
  key: string;
  name: string;
  /** Human-readable size string. */
  size: string;
}

export interface AttachmentBarProps {
  /** Called whenever the staged file list changes so the parent can read the count. */
  onCountChange?: (count: number) => void;
}

// ── Helpers ──────────────────────────────────────────────────────────────────

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

let keyCounter = 0;
function nextKey(): string {
  return `att-${++keyCounter}`;
}

// ── Component ────────────────────────────────────────────────────────────────

export function AttachmentBar({ onCountChange }: AttachmentBarProps) {
  const { t } = useTranslation("compose");
  const fileInputId = useId();
  const fileInputRef = useRef<HTMLInputElement>(null);

  const [staged, setStaged] = useState<StagedFile[]>([]);

  function addFiles(files: FileList | null) {
    if (!files || files.length === 0) return;
    const newItems: StagedFile[] = Array.from(files).map((f) => ({
      key: nextKey(),
      name: f.name,
      size: formatSize(f.size),
    }));
    const next = [...staged, ...newItems];
    setStaged(next);
    onCountChange?.(next.length);
    // Reset so the same file can be re-added after removal.
    if (fileInputRef.current) fileInputRef.current.value = "";
  }

  function removeFile(key: string) {
    const next = staged.filter((f) => f.key !== key);
    setStaged(next);
    onCountChange?.(next.length);
  }

  function handleDragOver(e: React.DragEvent<HTMLDivElement>) {
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
  }

  function handleDrop(e: React.DragEvent<HTMLDivElement>) {
    e.preventDefault();
    addFiles(e.dataTransfer.files);
  }

  return (
    <div
      onDragOver={handleDragOver}
      onDrop={handleDrop}
      className={cn(
        "flex flex-wrap items-center gap-2 border-t border-divider px-5 py-2.5",
        "transition-colors",
      )}
      aria-label="Attachments"
    >
      {/* Staged file chips */}
      {staged.map((file) => (
        <span
          key={file.key}
          className={cn(
            "inline-flex items-center gap-1.5 rounded-chip border border-divider",
            "bg-parchment px-2.5 py-1 font-ui text-[10px] text-p9",
          )}
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            viewBox="0 0 16 16"
            fill="currentColor"
            className="h-3 w-3 shrink-0 text-p7"
            aria-hidden
          >
            <path
              fillRule="evenodd"
              d="M4 2a2 2 0 00-2 2v8a2 2 0 002 2h8a2 2 0 002-2V6.414A2 2 0 0013.414 5L11 2.586A2 2 0 009.586 2H4zm4 7a1 1 0 10-2 0v1.5a.5.5 0 01-.5.5H5a1 1 0 100 2h1.5A2.5 2.5 0 009 10.5V9z"
              clipRule="evenodd"
            />
          </svg>
          <span className="max-w-[160px] truncate" title={file.name}>
            {file.name}
          </span>
          <span className="text-p7">{file.size}</span>
          <button
            type="button"
            onClick={() => removeFile(file.key)}
            aria-label={`Remove ${file.name}`}
            className="ms-0.5 rounded-full p-0.5 hover:bg-p5 hover:text-p10"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              viewBox="0 0 16 16"
              fill="currentColor"
              className="h-3 w-3"
              aria-hidden
            >
              <path d="M5.28 4.22a.75.75 0 00-1.06 1.06L6.94 8l-2.72 2.72a.75.75 0 101.06 1.06L8 9.06l2.72 2.72a.75.75 0 101.06-1.06L9.06 8l2.72-2.72a.75.75 0 00-1.06-1.06L8 6.94 5.28 4.22z" />
            </svg>
          </button>
        </span>
      ))}

      {/* Attach file button */}
      <label
        htmlFor={fileInputId}
        className={cn(
          "inline-flex cursor-pointer items-center gap-1.5 rounded-chip px-2.5 py-1",
          "font-ui text-[10px] uppercase tracking-wider text-p7",
          "transition-colors hover:bg-p4 hover:text-p10",
        )}
      >
        <svg
          xmlns="http://www.w3.org/2000/svg"
          viewBox="0 0 20 20"
          fill="currentColor"
          className="h-3.5 w-3.5"
          aria-hidden
        >
          <path
            fillRule="evenodd"
            d="M15.621 4.379a3 3 0 00-4.242 0l-7 7a3 3 0 004.241 4.243h.001l.497-.5a.75.75 0 011.064 1.057l-.498.501-.002.002a4.5 4.5 0 01-6.364-6.364l7-7a4.5 4.5 0 016.368 6.36l-3.455 3.553A2.625 2.625 0 119.52 9.52l3.45-3.451a.75.75 0 111.061 1.06l-3.45 3.451a1.125 1.125 0 001.587 1.595l3.454-3.553a3 3 0 000-4.242z"
            clipRule="evenodd"
          />
        </svg>
        {t("attach")}
      </label>
      <input
        ref={fileInputRef}
        id={fileInputId}
        type="file"
        multiple
        className="sr-only"
        onChange={(e) => addFiles(e.target.files)}
        tabIndex={-1}
        aria-hidden="true"
      />
    </div>
  );
}
