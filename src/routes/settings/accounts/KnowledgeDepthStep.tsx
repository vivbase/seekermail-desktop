// Knowledge-depth selection (T017 step 4). Six radio options with sampled counts
// (or "—" when sampling failed), "Last 12 months" recommended, and a large-mailbox
// warning. Token-styled; copy via i18n.
import { useTranslation } from "react-i18next";
import type { SamplingResult } from "@shared/bindings";

/** Depth buckets in months; `null` = all mail. */
const DEPTHS: (number | null)[] = [3, 6, 12, 36, 60, null];
const RECOMMENDED = 12;
const LARGE_MB_THRESHOLD = 100 * 1024; // 100 GB

interface KnowledgeDepthStepProps {
  sampling: SamplingResult | null;
  selected: number | null;
  onSelect: (months: number | null) => void;
}

export default function KnowledgeDepthStep({
  sampling,
  selected,
  onSelect,
}: KnowledgeDepthStepProps) {
  const { t } = useTranslation();

  const estimateFor = (months: number | null) => sampling?.ranges.find((r) => r.months === months);

  const allMail = estimateFor(null);
  const showLargeWarning = selected === null && (allMail?.estimatedMb ?? 0) > LARGE_MB_THRESHOLD;

  return (
    <div>
      <h3 className="font-display text-lg italic text-p10">{t("depth_title")}</h3>
      <ul className="mt-3 space-y-2">
        {DEPTHS.map((months) => {
          const est = estimateFor(months);
          const isSelected = selected === months;
          const key = months ?? "all";
          return (
            <li key={key}>
              <label
                className={`flex cursor-pointer items-center justify-between rounded-card border px-4 py-3 ${
                  isSelected ? "border-slate bg-p2" : "border-divider bg-surface"
                }`}
              >
                <span className="flex items-center gap-3">
                  <input
                    type="radio"
                    name="knowledge-depth"
                    checked={isSelected}
                    onChange={() => onSelect(months)}
                  />
                  <span className="font-body text-sm text-p9">
                    {months === null ? t("depth_all") : t("depth_months", { months })}
                  </span>
                  {months === RECOMMENDED && (
                    <span className="bg-sage/30 rounded-chip px-2 py-0.5 font-ui text-[10px] uppercase tracking-wider text-p9">
                      {t("depth_recommended")}
                    </span>
                  )}
                </span>
                <span className="font-mono text-xs text-p8">
                  {est?.mailCount != null
                    ? `${t("depth_count", { count: est.mailCount })} · ${t("depth_size", { mb: est.estimatedMb ?? 0 })}`
                    : t("depth_unknown")}
                </span>
              </label>
            </li>
          );
        })}
      </ul>
      {showLargeWarning && (
        <p className="bg-amber/15 mt-3 rounded-chip px-3 py-2 font-ui text-xs uppercase tracking-wider text-amber">
          {t("depth_large_warning")}
        </p>
      )}
    </div>
  );
}
