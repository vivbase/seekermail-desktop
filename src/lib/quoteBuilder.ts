// Quote-block and subject-prefix helpers for reply / forward modes (T044,
// F_G4 §4.7). Pure functions — no React, no side effects.

import type { MailDetail } from "@shared/bindings";

/** Format a Unix timestamp (seconds) as a readable date string for quote headers. */
function formatQuoteDate(unixSeconds: number): string {
  return new Intl.DateTimeFormat(undefined, {
    weekday: "short",
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(unixSeconds * 1000));
}

// ── Subject prefixes ─────────────────────────────────────────────────────────

/**
 * Return the "Re: …" subject for a reply, collapsing nested Re: prefixes so
 * "Re: Re: Budget" becomes "Re: Budget".
 */
export function buildReplySubject(original: string): string {
  const stripped = original.replace(/^(re:\s*)+/i, "").trim();
  return `Re: ${stripped}`;
}

/**
 * Return the "Fwd: …" subject for a forward, collapsing nested Fwd: prefixes.
 */
export function buildForwardSubject(original: string): string {
  const stripped = original.replace(/^(fwd?:\s*)+/i, "").trim();
  return `Fwd: ${stripped}`;
}

// ── Quote bodies ─────────────────────────────────────────────────────────────

/**
 * Build the plain-text quoted block appended below a reply body.
 *
 * Format:
 *   On Mon, Jun 1, 2026 at 09:42, Alice Nguyen <alice@example.com> wrote:
 *   > Original line 1
 *   > Original line 2
 */
export function buildReplyQuotePlain(mail: MailDetail): string {
  const sender = mail.fromName ? `${mail.fromName} <${mail.fromEmail}>` : mail.fromEmail;
  const header = `On ${formatQuoteDate(mail.dateSent)}, ${sender} wrote:`;
  const body = (mail.bodyText ?? "").trimEnd();
  const quoted = body
    .split("\n")
    .map((line) => `> ${line}`)
    .join("\n");
  return `\n\n${header}\n${quoted}`;
}

/**
 * Build the plain-text forwarded-message block.
 *
 * Format:
 *   ---------- Forwarded message ----------
 *   From: Alice Nguyen <alice@example.com>
 *   Date: Mon, Jun 1, 2026, 09:42
 *   Subject: Q4 budget review
 *   To: you@example.com
 *
 *   Original body…
 */
export function buildForwardBodyPlain(mail: MailDetail): string {
  const from = mail.fromName ? `${mail.fromName} <${mail.fromEmail}>` : mail.fromEmail;
  const to = mail.to.map((r) => (r.name ? `${r.name} <${r.email}>` : r.email)).join(", ");
  const lines = [
    "---------- Forwarded message ----------",
    `From: ${from}`,
    `Date: ${formatQuoteDate(mail.dateSent)}`,
    `Subject: ${mail.subject}`,
    `To: ${to}`,
    "",
    mail.bodyText ?? "",
  ];
  return `\n\n${lines.join("\n")}`;
}

// ── Seed builder ─────────────────────────────────────────────────────────────

export type ComposeSeed = {
  subject: string;
  to: string;
  cc: string;
  body: string;
  inReplyTo: string | null;
};

/**
 * Build the compose store seed for a reply (single sender, Cc omitted).
 */
export function buildReplySeed(mail: MailDetail): ComposeSeed {
  const toAddr = mail.fromEmail;
  return {
    subject: buildReplySubject(mail.subject),
    to: toAddr,
    cc: "",
    body: buildReplyQuotePlain(mail),
    inReplyTo: mail.id,
  };
}

/**
 * Build the compose store seed for reply-all (original sender + Cc recipients).
 */
export function buildReplyAllSeed(mail: MailDetail, ownEmail: string): ComposeSeed {
  const toAddrs = [mail.fromEmail, ...mail.cc.map((r) => r.email)]
    .filter((e) => e !== ownEmail)
    .join(", ");
  return {
    subject: buildReplySubject(mail.subject),
    to: toAddrs,
    cc: "",
    body: buildReplyQuotePlain(mail),
    inReplyTo: mail.id,
  };
}

/**
 * Build the compose store seed for a forward.
 */
export function buildForwardSeed(mail: MailDetail): ComposeSeed {
  return {
    subject: buildForwardSubject(mail.subject),
    to: "",
    cc: "",
    body: buildForwardBodyPlain(mail),
    inReplyTo: null,
  };
}
