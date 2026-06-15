// Inline query-card frame embedded in the TEAM message stream (T093). Visually
// mirrors the prototype `q-bubble`: a status label + query-type badge (T4 is the
// red alert tier) + a "View email" link + the question, with the action routed to
// the Pending DecisionCard (T099). The full interactive QA (options, submit, skip)
// lives on the Pending page — this never duplicates that logic; the inline option
// buttons depend on the query options, which the channel payload doesn't carry yet.

import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";

import type { ImMessage } from "@/ipc/im";
import { cn } from "@/lib/cn";

interface QueryCardEmbedProps {
  message: ImMessage;
}

/** Defensive parse of the query-card JSON content. Tolerates both camelCase
 *  (specta output) and snake_case (early hand-built cards). */
function parseCard(content: string): { questionText: string; triggerType: string } {
  try {
    const v = JSON.parse(content) as Record<string, unknown>;
    const questionText =
      (typeof v.questionText === "string" && v.questionText) ||
      (typeof v.question_text === "string" && v.question_text) ||
      (typeof v.text === "string" && v.text) ||
      "";
    const triggerType =
      (typeof v.triggerType === "string" && v.triggerType) ||
      (typeof v.trigger_type === "string" && v.trigger_type) ||
      "";
    return { questionText, triggerType };
  } catch {
    return { questionText: "", triggerType: "" };
  }
}

/** Left-border accent token: T4 is the red alert tier; other pending cards use the
 *  primary interactive tone; resolved cards fade to the divider tone. */
function accentVar(triggerType: string, pending: boolean): string {
  if (triggerType === "T4") return "var(--red)";
  return pending ? "var(--p9)" : "var(--p5)";
}

export default function QueryCardEmbed({ message }: QueryCardEmbedProps) {
  const { t } = useTranslation("team");
  const { questionText, triggerType } = parseCard(message.content);
  const pending = message.status === "pending";
  const isT4 = triggerType === "T4";
  const badgeLabel = triggerType
    ? t(`qa_type_${triggerType.toLowerCase()}`, { defaultValue: triggerType })
    : "";

  return (
    <div
      data-type="query_card_embed"
      className="rounded-card border border-divider bg-surface p-3 [border-inline-start-width:3px]"
      style={{ borderInlineStartColor: accentVar(triggerType, pending) }}
    >
      {/* Ref line: status label + query-type badge + View email */}
      <div className="flex flex-wrap items-center gap-2">
        <span className="section-label">
          {pending ? t("qa_card_pending_label") : t("qa_card_resolved_label")}
        </span>
        {badgeLabel && (
          <span
            className={cn(
              "rounded-chip px-1.5 py-0.5 font-ui text-[9px] font-semibold uppercase tracking-wider",
              isT4 ? "bg-red text-white" : "bg-p4 text-p9",
            )}
          >
            {badgeLabel}
          </span>
        )}
        {message.linkedEmailId && (
          <Link
            to={`/mail/${message.linkedEmailId}`}
            className="ms-auto font-ui text-[10px] uppercase tracking-wider text-p8 hover:text-p10 hover:underline"
          >
            {t("qa_view_email")}
          </Link>
        )}
      </div>
      {questionText && <p className="mt-1.5 font-body text-sm text-p10">{questionText}</p>}
      <Link
        to="/pending"
        className="mt-2 inline-block font-ui text-xs uppercase tracking-wider text-p9 hover:underline"
      >
        {t("qa_card_open_pending")}
      </Link>
    </div>
  );
}
