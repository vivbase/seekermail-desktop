// Reindex wizard (T053 §3b) — Settings → Data → Rebuild Index. Steps: account
// scope → confirm (sync pauses during rebuild) → live progress (gte:* stream)
// → completion report (read from the backend-persisted report setting). The
// /gte page's Start Reindex button calls the same IPC command.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";

import { useAccounts } from "@/ipc/queries/accounts";
import {
  useCancelReindex,
  useReindexProgress,
  useReindexReport,
  useStartReindex,
} from "@/ipc/queries/reindex";

type Step = 1 | 2 | 3 | 4;

export default function ReindexWizard() {
  const { t } = useTranslation("settings");

  const [step, setStep] = useState<Step>(1);
  const [accountId, setAccountId] = useState<string | null>(null);
  const [taskId, setTaskId] = useState<string | null>(null);

  const { data: accounts } = useAccounts();
  const startReindex = useStartReindex();
  const cancelReindex = useCancelReindex();
  const { progress, finished } = useReindexProgress(step === 3);
  const report = useReindexReport();

  const begin = () => {
    startReindex.mutate(accountId, {
      onSuccess: (id) => {
        setTaskId(id);
        setStep(3);
      },
    });
  };

  if (finished && step === 3) {
    report.refresh();
    setStep(4);
  }

  const indexed = progress?.indexed ?? 0;
  const pending = progress?.totalPending ?? 0;
  const total = indexed + pending;
  const pct = total > 0 ? Math.min(100, Math.round((indexed / total) * 100)) : 0;

  return (
    <div className="max-w-xl space-y-6">
      <div className="flex items-center justify-between">
        <p className="section-label">{t("reindex_title")}</p>
        <Link to="/settings/data" className="font-ui text-xs text-p8 hover:text-p9">
          {t("export_back_to_data")}
        </Link>
      </div>

      {step === 1 && (
        <section className="space-y-4">
          <div className="rounded-card border border-divider bg-surface px-4 py-4">
            <p className="font-ui text-sm font-medium text-p9">{t("reindex_scope")}</p>
            <div role="radiogroup" aria-label={t("reindex_scope")} className="mt-3 space-y-2">
              <label className="flex cursor-pointer items-center gap-3">
                <input
                  type="radio"
                  name="reindex-scope"
                  checked={accountId === null}
                  onChange={() => setAccountId(null)}
                  className="h-4 w-4"
                />
                <span className="font-body text-sm text-p9">{t("reindex_all_accounts")}</span>
              </label>
              {(accounts ?? []).map((acc) => (
                <label key={acc.id} className="flex cursor-pointer items-center gap-3">
                  <input
                    type="radio"
                    name="reindex-scope"
                    checked={accountId === acc.id}
                    onChange={() => setAccountId(acc.id)}
                    className="h-4 w-4"
                  />
                  <span className="font-body text-sm text-p9">{acc.displayName}</span>
                  <span className="font-mono text-xs text-p8">{acc.email}</span>
                </label>
              ))}
            </div>
          </div>
          <div className="flex justify-end">
            <button
              type="button"
              onClick={() => setStep(2)}
              className="rounded-chip bg-p9 px-4 py-1.5 font-ui text-sm font-medium text-p1 hover:bg-p10"
            >
              {t("export_next")}
            </button>
          </div>
        </section>
      )}

      {step === 2 && (
        <section className="space-y-4">
          <div className="rounded-card border border-divider bg-surface px-4 py-4">
            <p className="font-ui text-sm font-medium text-p9">{t("reindex_confirm_title")}</p>
            <p className="mt-2 font-body text-sm text-p9">{t("reindex_confirm_body")}</p>
            <p className="mt-2 rounded-chip bg-p4 px-3 py-2 font-body text-xs text-p8">
              {t("reindex_sync_paused_note")}
            </p>
          </div>
          <div className="flex justify-between">
            <button
              type="button"
              onClick={() => setStep(1)}
              className="rounded-chip px-4 py-1.5 font-ui text-sm text-p8 hover:text-p9"
            >
              {t("export_back")}
            </button>
            <button
              type="button"
              disabled={startReindex.isPending}
              onClick={begin}
              className="rounded-chip bg-p9 px-4 py-1.5 font-ui text-sm font-medium text-p1 hover:bg-p10 disabled:opacity-40"
            >
              {t("reindex_start")}
            </button>
          </div>
        </section>
      )}

      {step === 3 && (
        <section className="space-y-4">
          <div className="rounded-card border border-divider bg-surface px-4 py-4">
            <p className="font-ui text-sm font-medium text-p9">{t("reindex_in_progress")}</p>
            <div className="mt-3 h-2 overflow-hidden rounded-avatar bg-p4">
              <div
                role="progressbar"
                aria-valuenow={pct}
                aria-valuemin={0}
                aria-valuemax={100}
                className="h-full bg-green transition-all"
                style={{ width: `${pct}%` }}
              />
            </div>
            <p className="mt-2 font-mono text-xs text-p8">
              {progress
                ? t("reindex_progress_line", {
                    indexed: progress.indexed,
                    pending: progress.totalPending,
                  })
                : t("export_waiting")}
            </p>
          </div>
          <button
            type="button"
            onClick={() => taskId && cancelReindex.mutate(taskId)}
            className="rounded-chip border border-divider bg-parchment px-3 py-1.5 font-ui text-xs text-p9 hover:bg-p4"
          >
            {t("reindex_cancel")}
          </button>
          <p className="font-body text-xs text-p8">{t("reindex_cancel_note")}</p>
        </section>
      )}

      {step === 4 && (
        <section className="space-y-4">
          <div className="rounded-card border border-divider bg-surface px-4 py-4">
            <p className="font-ui text-sm font-medium text-green">{t("reindex_done")}</p>
            {report.data ? (
              <dl className="mt-3 space-y-1">
                <div className="flex justify-between">
                  <dt className="font-body text-sm text-p9">{t("reindex_report_processed")}</dt>
                  <dd className="font-mono text-sm text-p9">{report.data.processed}</dd>
                </div>
                <div className="flex justify-between">
                  <dt className="font-body text-sm text-p9">{t("reindex_report_verified")}</dt>
                  <dd className="font-mono text-sm text-p9">
                    {report.data.verifiedSample} /{" "}
                    {report.data.verifiedSample + report.data.verifyErrors}
                  </dd>
                </div>
                <div className="flex justify-between">
                  <dt className="font-body text-sm text-p9">{t("reindex_report_duration")}</dt>
                  <dd className="font-mono text-sm text-p9">
                    {Math.round(report.data.elapsedMs / 1000)}s
                  </dd>
                </div>
              </dl>
            ) : (
              <p className="mt-2 font-body text-sm text-p9">{t("reindex_report_pending")}</p>
            )}
          </div>
          <Link
            to="/settings/data"
            className="inline-block rounded-chip border border-divider px-4 py-1.5 font-ui text-sm text-p9 hover:bg-p4"
          >
            {t("export_done")}
          </Link>
        </section>
      )}
    </div>
  );
}
