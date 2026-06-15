// Sync-range page (T053 §3c) — Settings → Data → Sync Range. One row per
// account with the current history window and a Select to grow/shrink it.
// Shrinks show a confirm dialog with the exact local-delete count first
// (preview_sync_range); grows apply immediately and trigger a backfill.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";
import type { Account } from "@shared/bindings";

import { useAccounts } from "@/ipc/queries/accounts";
import { usePreviewSyncRange, useUpdateSyncRange } from "@/ipc/queries/reindex";

/** The selectable windows, in months. `null` = all history. */
const RANGE_OPTIONS: { months: number | null; labelKey: string }[] = [
  { months: 1, labelKey: "sync_range_30d" },
  { months: 3, labelKey: "sync_range_90d" },
  { months: 6, labelKey: "sync_range_180d" },
  { months: 12, labelKey: "sync_range_1y" },
  { months: null, labelKey: "sync_range_all" },
];

function optionValue(months: number | null): string {
  return months === null ? "all" : String(months);
}

interface PendingShrink {
  account: Account;
  months: number;
  beyond: number;
}

export default function SyncRangeSettings() {
  const { t } = useTranslation("settings");

  const { data: accounts } = useAccounts();
  const previewRange = usePreviewSyncRange();
  const updateRange = useUpdateSyncRange();
  const [pendingShrink, setPendingShrink] = useState<PendingShrink | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const apply = (account: Account, months: number | null) => {
    const current = account.knowledgeDepthMonths;
    const isShrink =
      months !== null && (current === null || current === undefined || months < current);
    if (isShrink) {
      previewRange.mutate(
        { accountId: account.id, months },
        {
          onSuccess: (p) => setPendingShrink({ account, months, beyond: p.mailsBeyondRange }),
        },
      );
      return;
    }
    updateRange.mutate(
      { accountId: account.id, months },
      { onSuccess: () => setNotice(t("sync_range_grow_started")) },
    );
  };

  const confirmShrink = () => {
    if (!pendingShrink) return;
    updateRange.mutate(
      { accountId: pendingShrink.account.id, months: pendingShrink.months },
      {
        onSuccess: (deleted) => setNotice(t("sync_range_shrink_done", { count: deleted })),
      },
    );
    setPendingShrink(null);
  };

  return (
    <div className="max-w-xl space-y-6">
      {pendingShrink && (
        <div
          className="bg-p10/30 fixed inset-0 z-50 flex items-center justify-center p-4"
          role="presentation"
        >
          <div
            className="w-full max-w-sm rounded-card bg-surface p-6 shadow-card"
            role="alertdialog"
            aria-modal="true"
            aria-label={t("sync_range_shrink_title")}
          >
            <p className="font-ui text-sm font-medium text-p10">{t("sync_range_shrink_title")}</p>
            <p className="mt-2 font-body text-sm text-p9">
              {t("sync_range_shrink_body", { count: pendingShrink.beyond })}
            </p>
            <div className="mt-5 flex justify-end gap-3">
              <button
                type="button"
                onClick={() => setPendingShrink(null)}
                className="rounded-chip px-4 py-1.5 font-ui text-sm text-p8 hover:text-p9"
              >
                {t("privacy_reset_cancel")}
              </button>
              <button
                type="button"
                onClick={confirmShrink}
                className="rounded-chip bg-red px-4 py-1.5 font-ui text-sm font-medium text-p1 hover:opacity-90"
              >
                {t("sync_range_shrink_confirm")}
              </button>
            </div>
          </div>
        </div>
      )}

      <div className="flex items-center justify-between">
        <p className="section-label">{t("sync_range_title")}</p>
        <Link to="/settings/data" className="font-ui text-xs text-p8 hover:text-p9">
          {t("export_back_to_data")}
        </Link>
      </div>

      {notice && (
        <p role="status" className="rounded-chip bg-p4 px-4 py-2 font-body text-xs text-p9">
          {notice}
        </p>
      )}

      <section className="flex flex-col gap-3">
        {(accounts ?? []).map((account) => (
          <div
            key={account.id}
            className="flex items-center justify-between gap-4 rounded-card border border-divider bg-surface px-4 py-4"
          >
            <div className="flex flex-col gap-0.5">
              <p className="font-ui text-sm font-medium text-p9">{account.displayName}</p>
              <p className="font-mono text-xs text-p8">{account.email}</p>
            </div>
            <select
              value={optionValue(account.knowledgeDepthMonths ?? null)}
              onChange={(e) => {
                const v = e.target.value;
                apply(account, v === "all" ? null : Number(v));
              }}
              aria-label={t("sync_range_select_label", { name: account.displayName })}
              className="rounded-chip border border-divider bg-surface px-3 py-1.5 font-ui text-sm text-p9"
            >
              {RANGE_OPTIONS.map((opt) => (
                <option key={optionValue(opt.months)} value={optionValue(opt.months)}>
                  {t(opt.labelKey)}
                </option>
              ))}
            </select>
          </div>
        ))}
      </section>

      <p className="font-body text-xs text-p8">{t("sync_range_note")}</p>
    </div>
  );
}
