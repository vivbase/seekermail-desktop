// Wipe wizard (T053 §3a) — Settings → Data → Wipe. Four steps: account +
// scope → impact preview → typed-DELETE confirmation → progress + freed space.
// The Confirm button stays disabled until the input matches "DELETE" exactly
// (case-sensitive). All backend access flows through the wipe hooks (07 §6).
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";
import type { WipePreview, WipeScope } from "@shared/bindings";

import { useAccounts } from "@/ipc/queries/accounts";
import { usePreviewWipe, useStartWipe, useWipeProgress } from "@/ipc/queries/wipe";
import { formatBytes } from "@/lib/formatBytes";
import { cn } from "@/lib/cn";

type Step = 1 | 2 | 3 | 4;

/** The exact confirmation string (case-sensitive, T053 §6). */
const CONFIRM_WORD = "DELETE";

const SCOPE_OPTIONS: { value: WipeScope; labelKey: string; descKey: string }[] = [
  { value: "mails_only", labelKey: "wipe_scope_mails", descKey: "wipe_scope_mails_desc" },
  {
    value: "mails_and_index",
    labelKey: "wipe_scope_mails_index",
    descKey: "wipe_scope_mails_index_desc",
  },
  { value: "everything", labelKey: "wipe_scope_everything", descKey: "wipe_scope_everything_desc" },
];

export default function WipeWizard() {
  const { t } = useTranslation("settings");

  const [step, setStep] = useState<Step>(1);
  const [selected, setSelected] = useState<string[]>([]);
  const [scope, setScope] = useState<WipeScope>("mails_and_index");
  const [confirmText, setConfirmText] = useState("");
  const [preview, setPreview] = useState<WipePreview | null>(null);
  const [taskId, setTaskId] = useState<string | null>(null);

  const { data: accounts } = useAccounts();
  const previewWipe = usePreviewWipe();
  const startWipe = useStartWipe();
  const { progress, complete } = useWipeProgress(taskId);

  const toggleAccount = (id: string) =>
    setSelected((prev) => (prev.includes(id) ? prev.filter((a) => a !== id) : [...prev, id]));

  const loadPreview = () => {
    previewWipe.mutate(selected, {
      onSuccess: (p) => {
        setPreview(p);
        setStep(2);
      },
    });
  };

  const begin = () => {
    startWipe.mutate(
      { accountIds: selected, scope },
      {
        onSuccess: (id) => {
          setTaskId(id);
          setStep(4);
        },
      },
    );
  };

  const confirmed = confirmText === CONFIRM_WORD;
  const pct =
    progress && progress.total > 0
      ? Math.min(100, Math.round((progress.deleted / progress.total) * 100))
      : complete
        ? 100
        : 0;

  return (
    <div className="max-w-xl space-y-6">
      <div className="flex items-center justify-between">
        <p className="section-label">{t("wipe_title")}</p>
        <Link to="/settings/data" className="font-ui text-xs text-p8 hover:text-p9">
          {t("export_back_to_data")}
        </Link>
      </div>

      {step === 1 && (
        <section className="space-y-4">
          <div className="rounded-card border border-divider bg-surface px-4 py-4">
            <p className="font-ui text-sm font-medium text-p9">{t("wipe_select_accounts")}</p>
            <div className="mt-3 space-y-2">
              {(accounts ?? []).map((acc) => (
                <label key={acc.id} className="flex cursor-pointer items-center gap-3">
                  <input
                    type="checkbox"
                    checked={selected.includes(acc.id)}
                    onChange={() => toggleAccount(acc.id)}
                    className="h-4 w-4"
                  />
                  <span className="font-body text-sm text-p9">{acc.displayName}</span>
                  <span className="font-mono text-xs text-p8">{acc.email}</span>
                </label>
              ))}
            </div>
          </div>

          <div className="rounded-card border border-divider bg-surface px-4 py-4">
            <p className="font-ui text-sm font-medium text-p9">{t("wipe_scope")}</p>
            <div role="radiogroup" aria-label={t("wipe_scope")} className="mt-3 space-y-2">
              {SCOPE_OPTIONS.map((opt) => (
                <label key={opt.value} className="flex cursor-pointer items-start gap-3">
                  <input
                    type="radio"
                    name="wipe-scope"
                    checked={scope === opt.value}
                    onChange={() => setScope(opt.value)}
                    className="mt-0.5 h-4 w-4"
                  />
                  <span>
                    <span className="font-ui text-sm text-p9">{t(opt.labelKey)}</span>
                    <span className="block font-body text-xs text-p8">{t(opt.descKey)}</span>
                  </span>
                </label>
              ))}
            </div>
          </div>

          <div className="flex justify-end">
            <button
              type="button"
              disabled={selected.length === 0 || previewWipe.isPending}
              onClick={loadPreview}
              className="rounded-chip bg-p9 px-4 py-1.5 font-ui text-sm font-medium text-p1 hover:bg-p10 disabled:cursor-not-allowed disabled:opacity-40"
            >
              {t("wipe_preview_impact")}
            </button>
          </div>
        </section>
      )}

      {step === 2 && preview && (
        <section className="space-y-4">
          <div className="rounded-card border border-divider bg-surface px-4 py-4">
            <p className="font-ui text-sm font-medium text-p9">{t("wipe_impact")}</p>
            <dl className="mt-3 space-y-1">
              <div className="flex justify-between">
                <dt className="font-body text-sm text-p9">{t("wipe_impact_mails")}</dt>
                <dd className="font-mono text-sm text-p9">{preview.mailCount}</dd>
              </div>
              <div className="flex justify-between">
                <dt className="font-body text-sm text-p9">{t("wipe_impact_attachments")}</dt>
                <dd className="font-mono text-sm text-p9">{preview.attachmentCount}</dd>
              </div>
              <div className="flex justify-between">
                <dt className="font-body text-sm text-p9">{t("wipe_impact_space")}</dt>
                <dd className="font-mono text-sm text-p9">{formatBytes(preview.estimatedBytes)}</dd>
              </div>
            </dl>
            <p className="mt-3 rounded-chip bg-p4 px-3 py-2 font-body text-xs text-red">
              {t("wipe_irreversible")}
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
              onClick={() => setStep(3)}
              className="rounded-chip bg-red px-4 py-1.5 font-ui text-sm font-medium text-p1 hover:opacity-90"
            >
              {t("wipe_continue")}
            </button>
          </div>
        </section>
      )}

      {step === 3 && (
        <section className="space-y-4">
          <div className="rounded-card border border-divider bg-surface px-4 py-4">
            <p className="font-ui text-sm font-medium text-p9">{t("wipe_confirm_title")}</p>
            <p className="mt-2 font-body text-sm text-p9">{t("wipe_confirm_body")}</p>
            <input
              type="text"
              value={confirmText}
              onChange={(e) => setConfirmText(e.target.value)}
              aria-label={t("wipe_confirm_input")}
              placeholder={CONFIRM_WORD}
              autoComplete="off"
              spellCheck={false}
              className="mt-3 w-full rounded-chip border border-divider bg-surface px-3 py-2 font-mono text-sm text-p9 focus:outline focus:outline-2 focus:outline-red"
            />
          </div>
          <div className="flex justify-between">
            <button
              type="button"
              onClick={() => setStep(2)}
              className="rounded-chip px-4 py-1.5 font-ui text-sm text-p8 hover:text-p9"
            >
              {t("export_back")}
            </button>
            <button
              type="button"
              disabled={!confirmed || startWipe.isPending}
              onClick={begin}
              className={cn(
                "rounded-chip px-4 py-1.5 font-ui text-sm font-medium text-p1",
                "bg-red hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40",
              )}
            >
              {t("wipe_confirm_button")}
            </button>
          </div>
        </section>
      )}

      {step === 4 && (
        <section className="space-y-4">
          <div className="rounded-card border border-divider bg-surface px-4 py-4">
            <p className="font-ui text-sm font-medium text-p9">
              {complete ? t("wipe_done") : t("wipe_in_progress")}
            </p>
            <div className="mt-3 h-2 overflow-hidden rounded-avatar bg-p4">
              <div
                role="progressbar"
                aria-valuenow={pct}
                aria-valuemin={0}
                aria-valuemax={100}
                className="h-full bg-red transition-all"
                style={{ width: `${pct}%` }}
              />
            </div>
            {progress && !complete && (
              <p className="mt-2 font-mono text-xs text-p8">
                {progress.deleted} / {progress.total}
              </p>
            )}
            {complete && (
              <p className="mt-2 font-body text-sm text-green">
                {t("wipe_freed", { space: formatBytes(complete.freedBytes) })}
              </p>
            )}
          </div>
          {complete && (
            <Link
              to="/settings/data"
              className="inline-block rounded-chip border border-divider px-4 py-1.5 font-ui text-sm text-p9 hover:bg-p4"
            >
              {t("export_done")}
            </Link>
          )}
        </section>
      )}
    </div>
  );
}
