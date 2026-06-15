// D1 Legal Assistant panel (T071, F_D1 §5) — lives in the L2 ThreadDrawer's
// "Legal" tab. Flow: explicit "Analyze Legal Risk" trigger → lazy 24 h-cached
// query → overall badge, risk list (high first, click-to-highlight), key
// clauses, compliance advice, resident disclaimer. AI_PROVIDER_UNREACHABLE /
// FORBIDDEN degrade to a "configure a provider" hint linking to /agents.
import { useState, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";
import { useQueryClient } from "@tanstack/react-query";

import { normalizeIpcError } from "@/ipc/client";
import {
  LEGAL_LEVEL_COLOR_VAR,
  LEGAL_LEVEL_WEIGHT,
  LEGAL_OVERALL_COLOR_VAR,
  type LegalAnalysisResult,
  type LegalKeyClauses,
  type LegalRiskItem,
} from "@/ipc/legal";
import { riskKeys, useAnalyzeLegalRisk, useLegalAnalysis } from "@/ipc/queries/risk";
import { useSelection } from "@/stores/selection";
import { cn } from "@/lib/cn";

/** Finding text is clamped defensively even though the backend caps it. */
const FINDING_MAX_CHARS = 80;

/** Error codes that mean "no usable AI provider" rather than a transient fault. */
const NO_PROVIDER_CODES = new Set(["AI_PROVIDER_UNREACHABLE", "FORBIDDEN"]);

interface LegalSidebarProps {
  mailId: string;
}

export function LegalSidebar({ mailId }: LegalSidebarProps) {
  const { t } = useTranslation("legal");
  const qc = useQueryClient();

  // The laziness gate: nothing is fetched until the user asks once. If a fresh
  // verdict is already cached (tab reopened within 24 h), start enabled so the
  // result shows immediately with no loading state (T071 §7).
  const [requested, setRequested] = useState<boolean>(
    () => qc.getQueryData(riskKeys.legalAnalysis(mailId)) !== undefined,
  );

  const analysis = useLegalAnalysis(mailId, requested);
  const reanalyze = useAnalyzeLegalRisk();

  const result: LegalAnalysisResult | undefined = analysis.data;
  const pending = (requested && analysis.isPending) || reanalyze.isPending;
  const rawError = reanalyze.error ?? analysis.error;
  const error = !pending && !result && rawError ? normalizeIpcError(rawError) : null;

  return (
    <div className="flex h-full flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto p-4">
        {!requested && !result && (
          <div className="flex flex-col items-stretch gap-3">
            <p className="font-body text-xs leading-relaxed text-p8">{t("legal_intro")}</p>
            <AnalyzeButton label={t("legal_analyze_btn")} onClick={() => setRequested(true)} />
          </div>
        )}

        {pending && (
          <div className="flex items-center justify-center gap-2 py-8" role="status">
            <Spinner />
            <span className="font-ui text-xs uppercase tracking-wider text-p8">
              {t("legal_analyzing")}
            </span>
          </div>
        )}

        {error && NO_PROVIDER_CODES.has(error.code) && (
          <div className="rounded-card border border-divider bg-p4 p-4">
            <p className="font-body text-xs text-p9">{t("legal_no_provider")}</p>
            <Link
              to="/agents"
              className="mt-2 inline-block font-ui text-xs uppercase tracking-wider text-terra underline transition-colors hover:text-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
            >
              {t("legal_no_provider_link")}
            </Link>
          </div>
        )}

        {error && !NO_PROVIDER_CODES.has(error.code) && (
          <div className="rounded-card border border-divider bg-p4 p-4" role="alert">
            <p className="font-body text-xs text-red">{t("legal_analyze_failed")}</p>
            <button
              type="button"
              onClick={() => reanalyze.mutate({ mailId, forceNew: true })}
              className="mt-3 rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white transition-colors hover:bg-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
            >
              {t("legal_retry")}
            </button>
          </div>
        )}

        {result && !pending && <AnalysisView result={result} />}
      </div>

      {/* Resident disclaimer — always pinned at the panel bottom (F_D1 §5). */}
      <p className="border-t border-divider px-4 py-2 font-ui text-[9px] uppercase tracking-wider text-p8">
        {t("legal_disclaimer")}
      </p>
    </div>
  );
}

// ── Result rendering ──────────────────────────────────────────────────────────

function AnalysisView({ result }: { result: LegalAnalysisResult }) {
  const { t } = useTranslation("legal");

  const sortedRisks = [...result.riskList].sort(
    (a, b) => LEGAL_LEVEL_WEIGHT[a.level] - LEGAL_LEVEL_WEIGHT[b.level],
  );

  return (
    <div className="flex flex-col gap-5">
      {/* Overall badge */}
      <div>
        <p className="section-label mb-1.5">{t("legal_overall_label")}</p>
        <span
          className="inline-flex items-center gap-1.5 rounded-chip px-2.5 py-1 font-ui text-[10px] font-semibold uppercase tracking-wider text-white"
          style={{ background: LEGAL_OVERALL_COLOR_VAR[result.overallLevel] }}
        >
          {t(`legal_overall_${result.overallLevel}`)}
        </span>
      </div>

      {/* Risk list */}
      <section aria-label={t("legal_section_risks")}>
        <p className="section-label mb-2">{t("legal_section_risks")}</p>
        {sortedRisks.length === 0 ? (
          <p className="font-body text-xs text-p7">{t("legal_no_risks")}</p>
        ) : (
          <ul role="list" className="flex flex-col gap-2">
            {sortedRisks.map((risk, i) => (
              <RiskItemCard key={`${risk.type}-${i}`} risk={risk} />
            ))}
          </ul>
        )}
      </section>

      <KeyClausesSection clauses={result.keyClauses} />

      {/* Compliance advice */}
      {result.complianceAdvice.length > 0 && (
        <section aria-label={t("legal_section_advice")}>
          <p className="section-label mb-2">{t("legal_section_advice")}</p>
          <ol className="list-decimal space-y-1.5 ps-4 font-body text-xs leading-relaxed text-p9">
            {result.complianceAdvice.map((advice, i) => (
              <li key={i}>{advice}</li>
            ))}
          </ol>
        </section>
      )}
    </div>
  );
}

function RiskItemCard({ risk }: { risk: LegalRiskItem }) {
  const { t } = useTranslation("legal");
  const [expanded, setExpanded] = useState(false);

  const legalHighlightText = useSelection((s) => s.legalHighlightText);
  const setLegalHighlight = useSelection((s) => s.setLegalHighlight);
  const isHighlighted = legalHighlightText === risk.originalText;

  const finding =
    risk.finding.length > FINDING_MAX_CHARS
      ? `${risk.finding.slice(0, FINDING_MAX_CHARS)}…`
      : risk.finding;

  return (
    <li
      className="rounded-card border border-divider bg-surface [border-inline-start-color:var(--risk-accent)] [border-inline-start-width:3px]"
      style={{ "--risk-accent": LEGAL_LEVEL_COLOR_VAR[risk.level] } as CSSProperties}
    >
      <button
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((v) => !v)}
        className="w-full p-3 text-start transition-colors hover:bg-p4 focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-p9"
      >
        <span className="flex items-center gap-1.5">
          <span
            aria-hidden
            className="h-2 w-2 shrink-0 rounded-avatar"
            style={{ background: LEGAL_LEVEL_COLOR_VAR[risk.level] }}
          />
          <span className="font-ui text-[10px] font-semibold uppercase tracking-wider text-p8">
            {t(`legal_risk_type_${risk.type}`)}
          </span>
        </span>
        <span className="mt-1 block font-body text-xs leading-snug text-p9">{finding}</span>
      </button>

      {expanded && (
        <div className="border-t border-divider p-3 pt-2.5">
          <p className="font-ui text-[9px] uppercase tracking-wider text-p8">
            {t("legal_suggestion_label")}
          </p>
          <p className="mt-1 font-body text-xs leading-relaxed text-p9">{risk.suggestion}</p>
          {/* Clicking the excerpt toggles the body highlight (T071 §3.3). */}
          <button
            type="button"
            aria-pressed={isHighlighted}
            aria-label={t("legal_highlight_action")}
            onClick={() => setLegalHighlight(isHighlighted ? null : risk.originalText)}
            className={cn(
              "mt-2 block w-full rounded-chip border px-2.5 py-1.5 text-start font-body text-xs italic leading-snug transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
              isHighlighted
                ? "border-terra bg-p4 text-p10"
                : "border-divider text-p8 hover:bg-p4 hover:text-p10",
            )}
          >
            &ldquo;{risk.originalText}&rdquo;
          </button>
        </div>
      )}
    </li>
  );
}

/** Clause field order + i18n suffix (F_D1 §5). Empty fields never render. */
const CLAUSE_FIELDS: { key: keyof LegalKeyClauses; labelKey: string }[] = [
  { key: "payment", labelKey: "legal_clause_payment" },
  { key: "delivery", labelKey: "legal_clause_delivery" },
  { key: "liability", labelKey: "legal_clause_liability" },
  { key: "confidentiality", labelKey: "legal_clause_confidentiality" },
  { key: "disputeResolution", labelKey: "legal_clause_dispute_resolution" },
];

function KeyClausesSection({ clauses }: { clauses: LegalKeyClauses }) {
  const { t } = useTranslation("legal");
  const present = CLAUSE_FIELDS.filter(({ key }) => !!clauses[key]);
  if (present.length === 0) return null;

  return (
    <section aria-label={t("legal_section_clauses")}>
      <details className="group rounded-card border border-divider bg-surface">
        <summary className="cursor-pointer list-none p-3 transition-colors hover:bg-p4 focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-p9">
          <span className="flex items-center justify-between">
            <span className="section-label">{t("legal_section_clauses")}</span>
            <svg
              width="12"
              height="12"
              viewBox="0 0 16 16"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.5"
              aria-hidden="true"
              className="text-p7 transition-transform group-open:rotate-180"
            >
              <path strokeLinecap="round" strokeLinejoin="round" d="m4 6 4 4 4-4" />
            </svg>
          </span>
        </summary>
        <dl className="border-t border-divider p-3 pt-2.5">
          {present.map(({ key, labelKey }) => (
            <div key={key} className="py-1.5 first:pt-0 last:pb-0">
              <dt className="font-ui text-[9px] uppercase tracking-wider text-p8">{t(labelKey)}</dt>
              <dd className="mt-0.5 font-body text-xs leading-relaxed text-p9">{clauses[key]}</dd>
            </div>
          ))}
        </dl>
      </details>
    </section>
  );
}

// ── Small pieces ──────────────────────────────────────────────────────────────

function AnalyzeButton({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex items-center justify-center gap-1.5 rounded-chip bg-p9 px-3 py-2 font-ui text-xs uppercase tracking-wider text-white transition-colors hover:bg-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
    >
      <svg
        width="13"
        height="13"
        viewBox="0 0 16 16"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        aria-hidden="true"
      >
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          d="M8 1.5 9.7 6l4.5.3-3.5 2.9 1.2 4.4L8 11l-3.9 2.6 1.2-4.4L1.8 6.3 6.3 6 8 1.5Z"
        />
      </svg>
      {label}
    </button>
  );
}

function Spinner() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      aria-hidden="true"
      className="animate-spin text-p8"
    >
      <circle cx="8" cy="8" r="6" strokeOpacity="0.25" />
      <path strokeLinecap="round" d="M14 8a6 6 0 0 0-6-6" />
    </svg>
  );
}
