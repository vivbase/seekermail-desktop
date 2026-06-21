// Hand-written mirrors of the Rust legal-analysis DTOs (T070 `src-tauri/src/types.rs`
// Module D section) and the Module E risk-event contracts (dev/02 §Module E). The
// `list_risk_events` / `resolve_risk_event` backends are registered and live
// (commands::risk, T071; lib.rs); the client.ts mock layer is only the off-Tauri
// dev/test double. Once `pnpm gen:types` emits these into `@shared/bindings`, delete
// this file and import from there instead (field shapes follow the generated-bindings
// conventions: camelCase, `| null` optionals).

/** Severity of one identified risk item (D1 output schema, F_D1 §4.4). */
export type LegalRiskLevel = "high" | "medium" | "low";

/** Worst-of aggregate over `riskList` (T070 §3); `none` = no risks at all. */
export type LegalOverallLevel = "high" | "medium" | "low" | "none";

/** D1 risk category (F_D1 §4.4). Serialized as `type` on the wire. */
export type LegalRiskType =
  | "payment"
  | "delivery"
  | "liability"
  | "confidentiality"
  | "dispute"
  | "other";

/** One identified risk (F_D1 §4.4). `originalText` ≤ 120 chars; `finding` /
 *  `suggestion` ≤ 80 chars — enforced at parse time on the Rust side. */
export type LegalRiskItem = {
  level: LegalRiskLevel;
  type: LegalRiskType;
  originalText: string;
  finding: string;
  suggestion: string;
};

/** Standard key-clause extraction (F_D1 §4.4). Absent clauses are `null`. */
export type LegalKeyClauses = {
  payment: string | null;
  delivery: string | null;
  liability: string | null;
  confidentiality: string | null;
  disputeResolution: string | null;
};

/** Input to `analyze_legal_risk` (T070 §3). `forceNew = false` replays the
 *  cached analysis when one exists within the last 24 hours (F_D1 §4.5). */
export type AnalyzeLegalRiskParams = {
  mailId: string;
  forceNew: boolean;
};

/** The D1 legal analysis verdict returned to the frontend (T070 §3). */
export type LegalAnalysisResult = {
  /** `ai_decisions.id` of this analysis (the E7 audit row). */
  decisionId: string;
  mailId: string;
  accountId: string;
  riskList: LegalRiskItem[];
  keyClauses: LegalKeyClauses;
  complianceAdvice: string[];
  /** Derived: the worst `riskList[].level` (T070 §6). */
  overallLevel: LegalOverallLevel;
  aiModel: string;
  /** Mail ids of the GTE chunks that grounded the analysis (dev/06 §9). */
  knowledgeRefs: string[];
  createdAt: number;
};

// ── Module E — risk events (dev/02 §Module E; dev/01 `risk_events`) ──────────

export type RiskStatus = "open" | "resolved" | "dismissed" | "expired";

/** Row DTO for `risk_events` (dev/02 §1). `riskLevel` is 1–6 (T1–T6);
 *  `expiresAt` is `null` for T4 — those never expire (AI_MODES_DESIGN §8.1). */
export type RiskEvent = {
  id: string;
  mailId: string;
  accountId: string;
  riskLevel: number;
  riskType: string;
  evidence: Record<string, unknown>;
  description: string;
  status: RiskStatus;
  expiresAt: number | null;
  createdAt: number;
};

/** Filter for `list_risk_events`. `mailId` extends the dev/02 contract — the
 *  T071 banner needs per-mail filtering; flag this when the backend card lands. */
export type ListRiskEventsParams = {
  accountId?: string;
  mailId?: string;
  status?: RiskStatus;
  riskLevel?: number;
};

/** Input to `resolve_risk_event` (dev/02 §Module E). */
export type ResolveRiskParams = {
  id: string;
  status: "resolved" | "dismissed";
  resolutionNote?: string;
};

/** T4 = level 4 in `risk_events.risk_level` — the non-dismissable tier. */
export const T4_RISK_LEVEL = 4;

/** Sort weight for D1 risk items: high first (F_D1 §5). */
export const LEGAL_LEVEL_WEIGHT: Record<LegalRiskLevel, number> = {
  high: 0,
  medium: 1,
  low: 2,
};

/** Design-token color per D1 risk level — never bare hex (root CLAUDE.md). */
export const LEGAL_LEVEL_COLOR_VAR: Record<LegalRiskLevel, string> = {
  high: "var(--terra)",
  medium: "var(--amber)",
  low: "var(--green)",
};

/** Token color for the overall badge; `none` falls back to the muted tone. */
export const LEGAL_OVERALL_COLOR_VAR: Record<LegalOverallLevel, string> = {
  high: "var(--terra)",
  medium: "var(--amber)",
  low: "var(--green)",
  none: "var(--p7)",
};

/** Token color for a numeric `risk_events.risk_level` (1–6). T4 and above use
 *  the terra alert tone; T3 amber; lower tiers the calm green. */
export function riskEventLevelColorVar(level: number): string {
  if (level >= T4_RISK_LEVEL) return "var(--terra)";
  if (level === 3) return "var(--amber)";
  return "var(--green)";
}
