// Event-type chip color map (T089 §6). Token CSS variables only — never hex.
// Unknown / future backend decision types fall back to the neutral p9 token.

export const EVENT_COLOR: Record<string, string> = {
  draft_sent: "var(--green)",
  e3_auto_sent: "var(--green)",
  auto_reply_sent: "var(--green)",
  draft_discarded: "var(--p7)",
  draft_expired: "var(--p7)",
  risk_intercepted: "var(--terra)",
  e4_sensitive: "var(--terra)",
  trust_downgraded: "var(--amber)",
};

/** Chip background for a decision type; neutral token for unknown types. */
export function eventColorVar(decisionType: string): string {
  return EVENT_COLOR[decisionType] ?? "var(--p9)";
}

/**
 * Human-readable label for a decision-type slug. The slugs are stable wire
 * identifiers (dev/01 §ai_decisions); product copy is English-only, so the
 * label is derived rather than duplicated across 21 locale files.
 */
export function eventTypeLabel(decisionType: string): string {
  return decisionType
    .split("_")
    .map((part) => (part.length <= 2 ? part.toUpperCase() : part[0]!.toUpperCase() + part.slice(1)))
    .join(" ");
}

/** Decision types that accept a mis-send report (T089 §3). */
export function supportsMisSendReport(decisionType: string): boolean {
  return (
    decisionType === "draft_sent" ||
    decisionType === "e3_auto_sent" ||
    decisionType === "auto_reply_sent"
  );
}
