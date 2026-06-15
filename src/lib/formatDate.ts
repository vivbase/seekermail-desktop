// Relative / short date formatters for mail timestamps (T037, F_G1 §4.1).
// All functions accept a Unix timestamp in seconds (as stored in the DB).
// Formatting follows the user's system locale and timezone via Intl — never
// hard-coded locale strings.

/**
 * Format a mail timestamp for the card stream column (T037 spec):
 *  - Same calendar day  → "HH:MM"  (e.g. "09:42")
 *  - Any earlier day    → "MMM D"  (e.g. "Jun 1", "Mar 14")
 */
export function formatMailDate(unixSeconds: number): string {
  const date = new Date(unixSeconds * 1000);
  const now = new Date();

  const isSameDay =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate();

  if (isSameDay) {
    return new Intl.DateTimeFormat(undefined, {
      hour: "2-digit",
      minute: "2-digit",
    }).format(date);
  }

  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
  }).format(date);
}

/**
 * Relative label for display in expanded / compact contexts:
 *  - Same day     → "HH:MM"
 *  - Yesterday    → "Yesterday"
 *  - Within 7 d   → day-of-week abbreviated (e.g. "Mon")
 *  - Older        → "MMM D" (e.g. "Jun 1")
 */
export function formatRelativeDate(unixSeconds: number): string {
  const date = new Date(unixSeconds * 1000);
  const now = new Date();

  const msPerDay = 86_400_000;
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const startOfDate = new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
  const daysDiff = Math.round((startOfToday - startOfDate) / msPerDay);

  if (daysDiff === 0) {
    return new Intl.DateTimeFormat(undefined, {
      hour: "2-digit",
      minute: "2-digit",
    }).format(date);
  }

  if (daysDiff === 1) {
    return "Yesterday";
  }

  if (daysDiff < 7) {
    return new Intl.DateTimeFormat(undefined, { weekday: "short" }).format(date);
  }

  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
  }).format(date);
}
