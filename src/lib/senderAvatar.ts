// Per-sender avatar derivation (T037, F_G1 §4.1). The L0 mail-card avatar
// identifies the *sender / correspondent*, not the account — so a single-account
// inbox still shows a distinct mark per sender (the old code reused the account
// badge, which made every row identical). The initial is the first letter of the
// sender's email address; the circle fill is a deterministic pick from the
// dashboard panel palette (terra / slate / sage / amber), so the same sender is
// always the same color while different senders stay visually separable.
//
// Color is local and deterministic — no Gravatar, no network — matching the
// privacy stance documented in AgentAvatar.tsx. A real fetched avatar image, when
// one becomes available, is layered on top by <SenderAvatar> via its `avatarUrl`.
import type { AccountColorToken } from "./accountColor";

// The four dashboard `stat-card` accent tokens (07 §7). The neutral `team`/`p9`
// token is intentionally excluded so every sender lands on a saturated panel color.
const SENDER_PALETTE: readonly AccountColorToken[] = ["terra", "slate", "sage", "amber"];

/** djb2-style hash folded to an unsigned 32-bit int (mirrors AgentAvatar §6). */
function hashString(input: string): number {
  let h = 5381;
  for (let i = 0; i < input.length; i += 1) {
    h = ((h << 5) + h + input.charCodeAt(i)) | 0;
  }
  return h >>> 0;
}

/** First alphanumeric character of a string, upper-cased; null when none exists. */
function firstAlnum(value: string): string | null {
  for (const ch of value.trim()) {
    if (/[a-z0-9]/i.test(ch)) return ch.toUpperCase();
  }
  return null;
}

/**
 * Deterministic dashboard-panel color token for a sender. Seeded by the normalized
 * email so the same correspondent always resolves to the same panel color.
 */
export function senderColorToken(email: string): AccountColorToken {
  const key = email.trim().toLowerCase();
  if (key.length === 0) return "slate";
  // The modulo is always an in-range index; `?? "slate"` only satisfies the
  // noUncheckedIndexedAccess type guard (the fallback is unreachable at runtime).
  return SENDER_PALETTE[hashString(key) % SENDER_PALETTE.length] ?? "slate";
}

/**
 * Avatar initial for a sender: the first letter of the email address, upper-cased.
 * Falls back to the display name, then to a neutral placeholder, when the email is
 * blank or has no alphanumeric character.
 */
export function senderInitial(email: string | null | undefined, name?: string | null): string {
  return firstAlnum(email ?? "") ?? firstAlnum(name ?? "") ?? "?";
}
