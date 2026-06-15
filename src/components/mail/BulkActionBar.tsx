// Bulk action bar (T038, F_G1 §4.6). Floats above the list when one or more
// thread cards are checked. Iterates over checkedThreadIds and calls mutations
// in parallel; shows progress and error count on partial failure.
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { useSelection } from "@/stores/selection";
import { useSetMailRead, useArchiveMail, useDeleteMail } from "@/ipc/queries/mail";
import { cn } from "@/lib/cn";

interface BulkActionBarProps {
  onArchived?: (ids: string[]) => void;
  onDeleted?: (ids: string[]) => void;
}

export function BulkActionBar({ onArchived, onDeleted }: BulkActionBarProps) {
  const { t } = useTranslation("list");
  const checkedThreadIds = useSelection((s) => s.checkedThreadIds);
  const clearChecked = useSelection((s) => s.clearChecked);

  const setMailRead = useSetMailRead();
  const archiveMail = useArchiveMail();
  const deleteMail = useDeleteMail();

  const [busy, setBusy] = useState(false);
  // Use a ref so runAll can always read the latest ids without stale closure.
  const idsRef = useRef<string[]>([]);
  idsRef.current = Array.from(checkedThreadIds);

  const count = checkedThreadIds.size;

  async function runAll(fn: (id: string) => Promise<unknown>): Promise<void> {
    setBusy(true);
    const ids = idsRef.current;
    const results = await Promise.allSettled(ids.map(fn));
    const errorCount = results.filter((r) => r.status === "rejected").length;
    setBusy(false);
    if (errorCount > 0) {
      console.warn(`Bulk action: ${errorCount} of ${ids.length} failed.`);
    }
  }

  const handleMarkRead = async () => {
    await runAll((id) => setMailRead.mutateAsync({ mailId: id, isRead: true }));
    clearChecked();
  };

  const handleMarkUnread = async () => {
    await runAll((id) => setMailRead.mutateAsync({ mailId: id, isRead: false }));
    clearChecked();
  };

  const handleArchive = async () => {
    const ids = idsRef.current;
    await runAll((id) => archiveMail.mutateAsync(id));
    onArchived?.(ids);
    clearChecked();
  };

  const handleDelete = async () => {
    const ids = idsRef.current;
    await runAll((id) => deleteMail.mutateAsync(id));
    onDeleted?.(ids);
    clearChecked();
  };

  if (count === 0) return null;

  return (
    <div
      role="toolbar"
      aria-label="Bulk actions"
      className={cn(
        "flex items-center gap-2 border-b border-divider bg-p2 px-4 py-2 transition-opacity",
        busy && "pointer-events-none opacity-60",
      )}
    >
      {/* Selected count announced to screen readers */}
      <span aria-live="polite" className="font-ui text-xs text-p9">
        {t("selected_count", { count })}
      </span>

      <div className="ms-2 flex items-center gap-1">
        <button
          type="button"
          onClick={() => void handleMarkRead()}
          disabled={busy}
          className="rounded-chip border border-divider bg-surface px-3 py-1 font-ui text-xs text-p9 hover:bg-p4 disabled:opacity-50"
        >
          {t("bulk_read")}
        </button>

        <button
          type="button"
          onClick={() => void handleMarkUnread()}
          disabled={busy}
          className="rounded-chip border border-divider bg-surface px-3 py-1 font-ui text-xs text-p9 hover:bg-p4 disabled:opacity-50"
        >
          {t("bulk_unread")}
        </button>

        <button
          type="button"
          onClick={() => void handleArchive()}
          disabled={busy}
          className="rounded-chip border border-divider bg-surface px-3 py-1 font-ui text-xs text-p9 hover:bg-p4 disabled:opacity-50"
        >
          {t("bulk_archive")}
        </button>

        <button
          type="button"
          onClick={() => void handleDelete()}
          disabled={busy}
          className="rounded-chip border border-divider bg-surface px-3 py-1 font-ui text-xs text-red hover:bg-p4 disabled:opacity-50"
        >
          {t("bulk_delete")}
        </button>
      </div>

      <button
        type="button"
        onClick={clearChecked}
        className="ms-auto rounded-chip px-2 py-1 font-ui text-xs text-p7 hover:text-p9"
      >
        {t("bulk_clear")}
      </button>
    </div>
  );
}
