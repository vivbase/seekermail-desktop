// E7 export control (T089 §3, F_E7 §4.7): CSV / JSON dropdown. The success
// toast shows only the FILENAME (never the full path — avoids leaking the
// directory layout); FS_DISK_FULL gets its dedicated message.
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { showToast } from "@/components/ui/Toast";
import { useExportAiDecisions, type AuditListFilters } from "@/ipc/queries/audit";
import { cn } from "@/lib/cn";

interface ExportButtonProps {
  filters: AuditListFilters;
  /** Default window applied when the date filter is unset. */
  defaultSinceUnix: number;
  defaultUntilUnix: number;
}

export function ExportButton({ filters, defaultSinceUnix, defaultUntilUnix }: ExportButtonProps) {
  const { t } = useTranslation("audit");
  const exportDecisions = useExportAiDecisions();
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);

  // Close the menu on outside click.
  useEffect(() => {
    if (!open) return;
    function onPointerDown(e: PointerEvent) {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) setOpen(false);
    }
    window.addEventListener("pointerdown", onPointerDown);
    return () => window.removeEventListener("pointerdown", onPointerDown);
  }, [open]);

  function runExport(format: "csv" | "json") {
    setOpen(false);
    exportDecisions.mutate(
      {
        accountId: filters.accountIds.length === 1 ? (filters.accountIds[0] ?? null) : null,
        sinceUnix: filters.sinceUnix ?? defaultSinceUnix,
        untilUnix: filters.untilUnix ?? defaultUntilUnix,
        format,
      },
      {
        onSuccess: (path) => {
          const filename = path.split("/").pop() ?? path;
          showToast(t("audit_export_success", { filename }));
        },
        onError: (err) => {
          showToast(
            err.code === "FS_DISK_FULL" ? t("audit_export_failed_disk") : t("audit_export_failed"),
          );
        },
      },
    );
  }

  return (
    <div ref={wrapRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        disabled={exportDecisions.isPending}
        aria-busy={exportDecisions.isPending}
        aria-haspopup="menu"
        aria-expanded={open}
        className={cn(
          "flex items-center gap-2 rounded-chip border border-divider px-3 py-1.5 font-ui text-[10px] font-semibold uppercase tracking-wider text-p9 transition-colors",
          "hover:bg-p4 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9 disabled:opacity-50",
        )}
      >
        {exportDecisions.isPending && (
          <span
            aria-hidden="true"
            className="inline-block h-3 w-3 animate-spin rounded-avatar border border-p7 border-t-transparent"
          />
        )}
        {t("audit_export")}
      </button>

      {open && (
        <div
          role="menu"
          aria-label={t("audit_export")}
          className="absolute z-10 mt-1 w-44 rounded-card border border-divider bg-surface p-1 shadow-card"
          style={{ insetInlineEnd: 0 }}
        >
          <button
            type="button"
            role="menuitem"
            onClick={() => runExport("csv")}
            className="block w-full rounded-chip px-3 py-2 text-start font-ui text-xs text-p9 transition-colors hover:bg-p4 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
          >
            {t("audit_export_csv")}
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => runExport("json")}
            className="block w-full rounded-chip px-3 py-2 text-start font-ui text-xs text-p9 transition-colors hover:bg-p4 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
          >
            {t("audit_export_json")}
          </button>
        </div>
      )}
    </div>
  );
}
