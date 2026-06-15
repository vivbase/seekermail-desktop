// Read-only diff of an AI draft (T090, F_E6 §4.5): bodyOriginal vs the user's
// current edit. Rendered as React elements (never dangerouslySetInnerHTML):
// insertions as <ins> on a green token tint, deletions as struck-through
// <del> in terra. Equal inputs show the "No edits made" hint row instead.
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import { diffText, hasChanges } from "@/lib/textDiff";

interface DraftDiffViewProps {
  /** AI-generated body (plain text, immutable reference). */
  original: string;
  /** Current (possibly user-edited) body, plain text. */
  current: string;
}

export function DraftDiffView({ original, current }: DraftDiffViewProps) {
  const { t } = useTranslation("aiDrafts");

  const segments = useMemo(() => diffText(original, current), [original, current]);
  const edited = useMemo(() => hasChanges(segments), [segments]);

  if (!edited) {
    return (
      <div
        data-testid="draft-diff-view"
        className="w-full rounded-card border border-divider bg-p1 p-3"
      >
        <p className="font-body text-sm italic text-p7">{t("draft_no_edits_hint")}</p>
      </div>
    );
  }

  return (
    <div
      data-testid="draft-diff-view"
      aria-label={t("draft_diff_label")}
      className="w-full whitespace-pre-wrap rounded-card border border-divider bg-p1 p-3 font-body text-sm leading-relaxed text-p10"
    >
      {segments.map((seg, index) => {
        if (seg.op === "insert") {
          return (
            <ins key={index} className="bg-green/20 text-p10 no-underline">
              {seg.text}
            </ins>
          );
        }
        if (seg.op === "delete") {
          return (
            <del key={index} className="text-terra line-through opacity-60">
              {seg.text}
            </del>
          );
        }
        return <span key={index}>{seg.text}</span>;
      })}
    </div>
  );
}
