// Hand-written mirrors of the Rust proactive-query + QA-card DTOs (T095/T098,
// `src-tauri/src/types.rs` + `ai/qa_card.rs`) until `pnpm gen:types` emits them
// into `@shared/bindings`. camelCase, `| null` optionals.

export type QueryStatus = "pending" | "answered" | "skipped" | "expired";

/** One proactive query lifecycle row (`pending_queries`). `options` carries the
 *  full QA card JSON (see {@link QaCardContent}) so the DecisionCard is
 *  self-contained from `list_pending_queries`. */
export type PendingQuery = {
  id: string;
  accountId: string;
  mailId: string | null;
  riskEventId: string | null;
  triggerType: string;
  question: string;
  options: string | null;
  answer: string | null;
  status: QueryStatus;
  priority: number;
  expiresAt: number | null;
  answeredAt: number | null;
  createdAt: number;
};

export type QaCardOption = { id: string; label: string; value: string };

export type QaCardSubQuestion = {
  questionText: string;
  options: QaCardOption[];
  multiSelect: boolean;
};

export type QaCardResponse = {
  selectedOptionIds: string[];
  freeText: string | null;
  submittedAt: number;
  actionResult: string | null;
};

export type QaCardContent = {
  cardVersion: number;
  linkedQueryId: string;
  triggerType: string;
  priority: string;
  linkedEmailId: string | null;
  questionText: string;
  options: QaCardOption[];
  multiSelect: boolean;
  freeTextPlaceholder: string | null;
  subQuestions: QaCardSubQuestion[];
  response: QaCardResponse | null;
};

/** Stable option values shared with the backend (`ai/qa_card.rs`). */
export const SKIP_VALUE = "__skip__";
export const VIEW_EMAIL_VALUE = "__view_email__";

/** Parse a query's `options` JSON into a QA card; `null` on any failure. */
export function parseQaCard(optionsJson: string | null): QaCardContent | null {
  if (!optionsJson) return null;
  try {
    const v = JSON.parse(optionsJson) as Partial<QaCardContent>;
    if (!Array.isArray(v.options)) return null;
    return v as QaCardContent;
  } catch {
    return null;
  }
}
