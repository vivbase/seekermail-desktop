// A compact, honest one-line summary of how the agent grounded its answer —
// what was searched, how much of the mailbox is indexed, and how many emails the
// answer used (analysis/54 §3.4, P-3 "state legibility"). This kills the "silent
// empty": the operator always sees whether mail was searched and why results may
// be thin, instead of reading an empty answer as "I have no such mail".
//
// Tone:
//   • warn (red)    → the semantic index could not be searched at all
//   • caution (amber) → the index is still building (partial coverage)
//   • muted (p7)    → normal: a grounded count, or "no matches" over a full index

import { useTranslation } from "react-i18next";

import type { RetrievalState } from "@/ipc/im";

interface RetrievalStateChipProps {
  state: RetrievalState;
}

export default function RetrievalStateChip({ state }: RetrievalStateChipProps) {
  const { t } = useTranslation("team");

  const grounded = state.semanticHits + state.temporalHits + state.memoryHits;
  // The semantic index only limits a topic/semantic answer; counts, recent mail,
  // and thread summaries read the full store, so the coverage caution applies
  // only to the semantic path.
  const semanticPath =
    state.aggregateFacts === 0 && state.temporalHits === 0 && state.memoryHits === 0;
  const partial = state.totalMails > 0 && state.indexedMails < state.totalMails;

  let tone: "warn" | "caution" | "muted";
  let label: string;
  if (!state.semanticAvailable) {
    tone = "warn";
    label = t("team_retrieval_unavailable");
  } else if (semanticPath && partial) {
    tone = "caution";
    label = t("team_retrieval_partial", {
      indexed: state.indexedMails,
      total: state.totalMails,
    });
  } else if (grounded === 0 && state.aggregateFacts === 0) {
    tone = "muted";
    label = t("team_retrieval_none");
  } else {
    tone = "muted";
    label = t("team_retrieval_grounded", { count: grounded });
  }

  const toneClass = tone === "warn" ? "text-red" : tone === "caution" ? "text-amber" : "text-p7";

  return (
    <p
      className={`mt-1 font-ui text-[10px] uppercase tracking-wide ${toneClass}`}
      title={t("team_retrieval_title")}
    >
      {label}
    </p>
  );
}
