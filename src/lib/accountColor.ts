// Account color-coding (07 §7, root CLAUDE.md). Maps an account's color token to
// a Tailwind class pair — never hardcoded per component. The Account DTO arrives
// at v0.2; this local union mirrors the schema's `color_token` plus the role
// accents so the helper is ready to consume it.
export type AccountColorToken = "terra" | "slate" | "sage" | "amber" | "team";

const TOKEN_CLASS: Record<AccountColorToken, string> = {
  terra: "bg-terra text-white", // Legal / high-risk
  slate: "bg-slate text-white", // Work
  sage: "bg-sage text-p10", // Personal
  amber: "bg-amber text-white", // Backup / warnings
  team: "bg-p9 text-white", // Team / shared
};

/** Tailwind classes for an account avatar/accent from its color token. */
export function accountColorClass(token: AccountColorToken): string {
  return TOKEN_CLASS[token];
}
