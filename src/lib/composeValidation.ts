// Pre-send validation helpers (T044, F_G4 §4.8). Pure functions — no side
// effects, no React, no IPC. Consumed by ComposeFooter before calling send_mail.

import type { Recipient } from "@shared/bindings";

// ── Recipient parsing ────────────────────────────────────────────────────────

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/** Returns true if the string looks like a plausible RFC-5322 address. */
export function isValidEmail(s: string): boolean {
  return EMAIL_RE.test(s.trim());
}

/**
 * Split a raw comma/semicolon-delimited recipient string into Recipient
 * objects. Entries that look like "Name <email>" are parsed; plain addresses
 * use null for name. Empty tokens are skipped.
 */
export function parseRecipients(raw: string): Recipient[] {
  if (!raw.trim()) return [];

  return raw
    .split(/[,;]+/)
    .map((token) => token.trim())
    .filter(Boolean)
    .map((token) => {
      // "Display Name <email>" form
      const match = token.match(/^(.+?)\s*<([^>]+)>$/);
      if (match) {
        return { name: match[1]?.trim() || null, email: (match[2] ?? "").trim() };
      }
      return { name: null, email: token };
    });
}

// ── Validation result ────────────────────────────────────────────────────────

export type ValidationCode =
  | "NO_ACCOUNT"
  | "NO_RECIPIENT"
  | "INVALID_EMAIL"
  | "MANY_RECIPIENTS"
  | "FORGOT_ATTACHMENT"
  | "NO_SUBJECT";

export interface ValidationError {
  code: ValidationCode;
  /** Resolved human-readable message (interpolation already applied). */
  message: string;
  /** Whether this error is a hard blocker (true) or a soft warning (false). */
  blocking: boolean;
}

export interface ValidationResult {
  ok: boolean;
  /** Hard-blocking errors — send must not proceed. */
  errors: ValidationError[];
  /** Soft warnings — user may dismiss and send anyway. */
  warnings: ValidationError[];
}

interface ValidateInput {
  accountId: string | null;
  to: string;
  subject: string;
  body: string;
  attachmentCount: number;
}

const ATTACHMENT_HINT_RE =
  /\b(attach(ed|ment|ments)?|see attached|please find|file(s)? (attached|enclosed))\b/i;

/**
 * Multi-stage pre-send validation (F_G4 §4.8).
 *
 * Stage 1 — hard blockers:
 *   - No From account selected
 *   - No recipient addresses provided
 *   - One or more recipient addresses are malformed
 *
 * Stage 2 — soft warnings:
 *   - 10 or more recipients (2a)
 *   - Body mentions "attached" but no attachments staged (2b)
 *   - Subject is blank (2c)
 */
export function validateCompose(input: ValidateInput): ValidationResult {
  const errors: ValidationError[] = [];
  const warnings: ValidationError[] = [];

  // Stage 1a — account
  if (!input.accountId) {
    errors.push({
      code: "NO_ACCOUNT",
      message: "Choose an account to send from.",
      blocking: true,
    });
  }

  // Stage 1b — recipients
  const recipients = parseRecipients(input.to);
  if (recipients.length === 0) {
    errors.push({
      code: "NO_RECIPIENT",
      message: "Add at least one recipient.",
      blocking: true,
    });
  } else {
    // Stage 1c — address format
    const invalid = recipients.filter((r) => !isValidEmail(r.email));
    for (const r of invalid) {
      errors.push({
        code: "INVALID_EMAIL",
        message: `"${r.email}" doesn't look like a valid address.`,
        blocking: true,
      });
    }
  }

  // Stage 2a — many recipients
  if (recipients.length >= 10) {
    warnings.push({
      code: "MANY_RECIPIENTS",
      message: `You're sending to ${recipients.length} recipients. Continue?`,
      blocking: false,
    });
  }

  // Stage 2b — attachment hint without attachment
  if (ATTACHMENT_HINT_RE.test(input.body) && input.attachmentCount === 0) {
    warnings.push({
      code: "FORGOT_ATTACHMENT",
      message: "Your message mentions an attachment, but none are staged. Send anyway?",
      blocking: false,
    });
  }

  // Stage 2c — empty subject
  if (!input.subject.trim()) {
    warnings.push({
      code: "NO_SUBJECT",
      message: "Your message has no subject. Send anyway?",
      blocking: false,
    });
  }

  return {
    ok: errors.length === 0,
    errors,
    warnings,
  };
}
