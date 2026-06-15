// Export wizard (T052) — Settings → Data → Export. Four steps: account/date
// selection → format/content → progress (event-driven) → done (open in Finder).
// All backend access flows through the export hooks; progress comes from the
// `export:*` event stream, never polling (07 §6).
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";
import type { ExportFormat } from "@shared/bindings";

import { useAccounts } from "@/ipc/queries/accounts";
import {
  useCancelExport,
  useExportProgress,
  useOpenExportOutput,
  useStartExport,
} from "@/ipc/queries/export";
import { normalizeIpcError } from "@/ipc/client";
import { cn } from "@/lib/cn";

type Step = 1 | 2 | 3 | 4;

/** Parse a yyyy-mm-dd input value to unix seconds (UTC midnight), or null. */
function dateToUnix(value: string, endOfDay: boolean): number | null {
  if (!value) return null;
  const ms = Date.parse(`${value}T${endOfDay ? "23:59:59" : "00:00:00"}Z`);
  return Number.isNaN(ms) ? null : Math.floor(ms / 1000);
}

export default function ExportWizard() {
  const { t } = useTranslation("settings");

  const [step, setStep] = useState<Step>(1);
  const [selected, setSelected] = useState<string[]>([]);
  const [dateFrom, setDateFrom] = useState("");
  const [dateTo, setDateTo] = useState("");
  const [format, setFormat] = useState<ExportFormat>("mbox");
  const [includeBody, setIncludeBody] = useState(true);
  const [includeAttachments, setIncludeAttachments] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);

  const { data: accounts } = useAccounts();
  const startExport = useStartExport();
  const cancelExport = useCancelExport();
  const openOutput = useOpenExportOutput();
  const { progress, complete, error } = useExportProgress(taskId);

  const toggleAccount = (id: string) =>
    setSelected((prev) => (prev.includes(id) ? prev.filter((a) => a !== id) : [...prev, id]));

  const begin = () => {
    startExport.mutate(
      {
        accountIds: selected,
        dateFrom: dateToUnix(dateFrom, false),
        dateTo: dateToUnix(dateTo, true),
        format,
        includeBody,
        includeAttachments,
      },
      {
        onSuccess: (id) => {
          setTaskId(id);
          setStep(3);
        },
      },
    );
  };

  const pct =
    progress && progress.total > 0
      ? Math.min(100, Math.round((progress.count / progress.total) * 100))
      : 0;
  if (complete && step === 3) setStep(4);

  const startError = startExport.error ? normalizeIpcError(startExport.error) : null;

  return (
    <div className="max-w-xl space-y-6">
      <div className="flex items-center justify-between">
        <p className="section-label">{t("export_title")}</p>
        <Link to="/settings/data" className="font-ui text-xs text-p8 hover:text-p9">
          {t("export_back_to_data")}
        </Link>
      </div>

      <StepDots
        step={step}
        labels={[t("export_step1"), t("export_step2"), t("export_step3"), t("export_step4")]}
      />

      {step === 1 && (
        <section className="space-y-4">
          <div className="rounded-card border border-divider bg-surface px-4 py-4">
            <p className="font-ui text-sm font-medium text-p9">{t("export_select_accounts")}</p>
            <div className="mt-3 space-y-2">
              {(accounts ?? []).map((acc) => (
                <label key={acc.id} className="flex cursor-pointer items-center gap-3">
                  <input
                    type="checkbox"
                    checked={selected.includes(acc.id)}
                    onChange={() => toggleAccount(acc.id)}
                    className="h-4 w-4 text-p9 accent-current"
                  />
                  <span className="font-body text-sm text-p9">{acc.displayName}</span>
                  <span className="font-mono text-xs text-p8">{acc.email}</span>
                </label>
              ))}
              {(accounts ?? []).length === 0 && (
                <p className="font-body text-xs text-p8">{t("export_no_accounts")}</p>
              )}
            </div>
          </div>

          <div className="rounded-card border border-divider bg-surface px-4 py-4">
            <p className="font-ui text-sm font-medium text-p9">{t("export_date_range")}</p>
            <p className="mt-1 font-body text-xs text-p8">{t("export_date_range_desc")}</p>
            <div className="mt-3 flex items-center gap-3">
              <input
                type="date"
                value={dateFrom}
                onChange={(e) => setDateFrom(e.target.value)}
                aria-label={t("export_date_from")}
                className="rounded-chip border border-divider bg-surface px-3 py-1.5 font-mono text-xs text-p9"
              />
              <span className="font-ui text-xs text-p8">{t("export_date_to_sep")}</span>
              <input
                type="date"
                value={dateTo}
                onChange={(e) => setDateTo(e.target.value)}
                aria-label={t("export_date_to")}
                className="rounded-chip border border-divider bg-surface px-3 py-1.5 font-mono text-xs text-p9"
              />
            </div>
          </div>

          <WizardNav
            nextLabel={t("export_next")}
            nextDisabled={selected.length === 0}
            onNext={() => setStep(2)}
          />
        </section>
      )}

      {step === 2 && (
        <section className="space-y-4">
          <div className="rounded-card border border-divider bg-surface px-4 py-4">
            <p className="font-ui text-sm font-medium text-p9">{t("export_format")}</p>
            <div role="radiogroup" aria-label={t("export_format")} className="mt-3 flex gap-2">
              {(["mbox", "json"] as const).map((f) => (
                <button
                  key={f}
                  type="button"
                  role="radio"
                  aria-checked={format === f}
                  onClick={() => setFormat(f)}
                  className={cn(
                    "rounded-chip border px-3 py-1.5 font-ui text-sm transition-colors",
                    format === f
                      ? "border-transparent bg-p9 text-p1"
                      : "border-divider bg-surface text-p9 hover:bg-p4",
                  )}
                >
                  {f === "mbox" ? t("export_format_mbox") : t("export_format_json")}
                </button>
              ))}
            </div>
            <p className="mt-2 font-body text-xs text-p8">
              {format === "mbox" ? t("export_format_mbox_desc") : t("export_format_json_desc")}
            </p>
          </div>

          <div className="rounded-card border border-divider bg-surface px-4 py-4">
            <p className="font-ui text-sm font-medium text-p9">{t("export_content")}</p>
            <label className="mt-3 flex cursor-pointer items-center gap-3">
              <input
                type="checkbox"
                checked={includeBody}
                onChange={(e) => setIncludeBody(e.target.checked)}
                className="h-4 w-4"
              />
              <span className="font-body text-sm text-p9">{t("export_include_body")}</span>
            </label>
            <label className="mt-2 flex cursor-pointer items-center gap-3">
              <input
                type="checkbox"
                checked={includeAttachments}
                onChange={(e) => setIncludeAttachments(e.target.checked)}
                className="h-4 w-4"
              />
              <span className="font-body text-sm text-p9">{t("export_include_attachments")}</span>
            </label>
            <p className="mt-2 font-body text-xs text-p8">{t("export_credentials_note")}</p>
          </div>

          {startError && (
            <p role="alert" className="rounded-chip bg-p4 px-4 py-2 font-body text-xs text-red">
              {startError.code === "FS_DISK_FULL" ? t("export_disk_full") : t("export_failed")}
            </p>
          )}

          <WizardNav
            backLabel={t("export_back")}
            onBack={() => setStep(1)}
            nextLabel={startExport.isPending ? t("export_starting") : t("export_start")}
            nextDisabled={startExport.isPending}
            onNext={begin}
          />
        </section>
      )}

      {step === 3 && (
        <section className="space-y-4">
          <div className="rounded-card border border-divider bg-surface px-4 py-4">
            <p className="font-ui text-sm font-medium text-p9">{t("export_in_progress")}</p>
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
                ? `${progress.count} / ${progress.total} · ${progress.stage}`
                : t("export_waiting")}
            </p>
          </div>

          {error && (
            <p role="alert" className="rounded-chip bg-p4 px-4 py-2 font-body text-xs text-red">
              {error.code === "FS_DISK_FULL" ? t("export_disk_full") : t("export_failed")}
            </p>
          )}

          <button
            type="button"
            onClick={() => taskId && cancelExport.mutate(taskId)}
            className="rounded-chip border border-divider bg-parchment px-3 py-1.5 font-ui text-xs text-p9 hover:bg-p4"
          >
            {t("export_cancel")}
          </button>
        </section>
      )}

      {step === 4 && complete && (
        <section className="space-y-4">
          <div className="rounded-card border border-divider bg-surface px-4 py-4">
            <p className="font-ui text-sm font-medium text-green">{t("export_complete")}</p>
            <p className="mt-2 font-body text-sm text-p9">
              {t("export_complete_count", { count: complete.mailCount })}
            </p>
            <p className="mt-1 break-all font-mono text-xs text-p8">{complete.outputPath}</p>
          </div>
          <div className="flex gap-3">
            <button
              type="button"
              onClick={() => taskId && openOutput.mutate(taskId)}
              className="rounded-chip bg-p9 px-4 py-1.5 font-ui text-sm font-medium text-p1 hover:bg-p10"
            >
              {t("export_open_in_finder")}
            </button>
            <Link
              to="/settings/data"
              className="rounded-chip border border-divider px-4 py-1.5 font-ui text-sm text-p9 hover:bg-p4"
            >
              {t("export_done")}
            </Link>
          </div>
        </section>
      )}
    </div>
  );
}

// ── Sub-components ────────────────────────────────────────────────────────────

function StepDots({ step, labels }: { step: Step; labels: string[] }) {
  return (
    <ol className="flex items-center gap-4">
      {labels.map((label, i) => {
        const n = (i + 1) as Step;
        return (
          <li key={label} className="flex items-center gap-2">
            <span
              aria-current={n === step ? "step" : undefined}
              className={cn(
                "flex h-5 w-5 items-center justify-center rounded-avatar font-mono text-[10px]",
                n <= step ? "bg-p9 text-p1" : "bg-p4 text-p8",
              )}
            >
              {n}
            </span>
            <span className="font-ui text-xs text-p8">{label}</span>
          </li>
        );
      })}
    </ol>
  );
}

function WizardNav({
  backLabel,
  onBack,
  nextLabel,
  nextDisabled,
  onNext,
}: {
  backLabel?: string;
  onBack?: () => void;
  nextLabel: string;
  nextDisabled?: boolean;
  onNext: () => void;
}) {
  return (
    <div className="flex justify-between">
      {backLabel && onBack ? (
        <button
          type="button"
          onClick={onBack}
          className="rounded-chip px-4 py-1.5 font-ui text-sm text-p8 hover:text-p9"
        >
          {backLabel}
        </button>
      ) : (
        <span />
      )}
      <button
        type="button"
        disabled={nextDisabled}
        onClick={onNext}
        className="rounded-chip bg-p9 px-4 py-1.5 font-ui text-sm font-medium text-p1 hover:bg-p10 disabled:cursor-not-allowed disabled:opacity-40"
      >
        {nextLabel}
      </button>
    </div>
  );
}
