// Data & Storage hub (T052/T053/T054). Shows the local storage summary and
// routes to the data-management sub-pages: Export, Wipe, Rebuild Index,
// Sync Range, and the Data Flow disclosure panel.
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";

import { useAccounts, useDiskUsage } from "@/ipc/queries/accounts";
import { formatBytes } from "@/lib/formatBytes";

// ── Storage row ───────────────────────────────────────────────────────────────

function StorageRow({ label, bytes }: { label: string; bytes: number }) {
  return (
    <div className="flex items-center justify-between py-2">
      <span className="font-body text-sm text-p9">{label}</span>
      <span className="font-mono text-sm text-p8">{formatBytes(bytes)}</span>
    </div>
  );
}

// ── Hub link card ─────────────────────────────────────────────────────────────

function HubLink({
  to,
  title,
  description,
  destructive,
}: {
  to: string;
  title: string;
  description: string;
  destructive?: boolean;
}) {
  return (
    <Link
      to={to}
      className="group flex items-start justify-between gap-4 rounded-card border border-divider bg-surface px-4 py-4 transition-colors hover:bg-p4"
    >
      <div className="flex flex-col gap-0.5">
        <p className={`font-ui text-sm font-medium ${destructive ? "text-red" : "text-p9"}`}>
          {title}
        </p>
        <p className="font-body text-xs leading-relaxed text-p8">{description}</p>
      </div>
      <svg
        aria-hidden
        viewBox="0 0 16 16"
        className="mt-1 h-4 w-4 shrink-0 fill-none stroke-current text-p8 group-hover:text-p9 rtl:-scale-x-100"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <path d="M6 3l5 5-5 5" />
      </svg>
    </Link>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

export default function DataSettings() {
  const { t } = useTranslation("settings");

  const { data: accounts } = useAccounts();
  // Show disk usage for the first account as a representative figure.
  const primaryId = accounts?.[0]?.id ?? "";
  const { data: disk } = useDiskUsage(primaryId);

  const totalBytes = disk?.totalBytes ?? 0;
  const attachmentBytes = disk?.attachmentBytes ?? 0;
  const bodyBytes = disk?.bodyBytes ?? 0;
  const indexBytes = Math.max(0, totalBytes - attachmentBytes - bodyBytes);

  return (
    <div className="max-w-xl space-y-6">
      {/* Storage summary */}
      <section>
        <p className="section-label mb-3">{t("data_title")}</p>
        <div className="divide-y divide-divider rounded-card border border-divider bg-surface px-4">
          <StorageRow label={t("data_row_bodies")} bytes={bodyBytes} />
          <StorageRow label={t("data_row_attachments")} bytes={attachmentBytes} />
          <StorageRow label={t("data_row_index")} bytes={indexBytes} />
          <div className="flex items-center justify-between py-2">
            <span className="font-ui text-xs font-medium uppercase tracking-wider text-p8">
              {t("data_usage")}
            </span>
            <span className="font-mono text-sm font-medium text-p9">{formatBytes(totalBytes)}</span>
          </div>
        </div>
        <p className="mt-1 font-body text-xs text-p8">{t("data_usage_desc")}</p>
      </section>

      {/* Sub-page links */}
      <section className="flex flex-col gap-3">
        <HubLink
          to="/settings/data/export"
          title={t("data_link_export")}
          description={t("data_link_export_desc")}
        />
        <HubLink
          to="/settings/data/reindex"
          title={t("data_link_reindex")}
          description={t("data_link_reindex_desc")}
        />
        <HubLink
          to="/settings/data/sync-range"
          title={t("data_link_sync_range")}
          description={t("data_link_sync_range_desc")}
        />
        <HubLink
          to="/settings/data/data-flow"
          title={t("data_link_data_flow")}
          description={t("data_link_data_flow_desc")}
        />
        <HubLink
          to="/settings/data/wipe"
          title={t("data_link_wipe")}
          description={t("data_link_wipe_desc")}
          destructive
        />
      </section>
    </div>
  );
}
