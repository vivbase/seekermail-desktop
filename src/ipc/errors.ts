// ErrorCode → { bucket, messageKey, affordance } — the single typed table the UI
// renders from (T004, 09 §4). Keyed by the generated `ErrorCode` union via
// `Record<ErrorCode, …>`, so adding a backend code FAILS the build until a UX
// decision is made here (09 §8 "mapping completeness"). Components read this
// table; they never switch-case raw codes inline (07 §9).
import type { ErrorCode } from "@shared/bindings";

/** Taxonomy bucket — drives the default UX, not the code itself (09 §2). */
export type ErrorBucket =
  | "retryable"
  | "user_correctable"
  | "recoverable"
  | "forbidden"
  | "environment"
  | "internal"
  | "silent";

/** Recovery affordance the owning component renders. */
export type Affordance =
  | "retry"
  | "reenter_credentials"
  | "restart_oauth"
  | "open_keychain_guidance"
  | "run_full_sync"
  | "keep_draft"
  | "auto_backoff"
  | "guided_repair"
  | "switch_provider"
  | "trim_context"
  | "start_reindex"
  | "show_progress"
  | "os_guidance"
  | "free_space"
  | "inline"
  | "explain"
  | "refresh"
  | "report"
  | "none";

export interface ErrorUx {
  bucket: ErrorBucket;
  /** i18n key in the `errors` namespace (T008 fills the literals; never a literal here). */
  messageKey: string;
  affordance: Affordance;
}

/**
 * The mapping consumed by `07 §9`. Copy intents follow 09 §4; the literals live in
 * `src/i18n/resources/<locale>/errors.json` keyed by `messageKey`.
 */
export const ERROR_UX: Record<ErrorCode, ErrorUx> = {
  AUTH_INVALID_CREDENTIALS: {
    bucket: "user_correctable",
    messageKey: "err_auth_invalid_credentials",
    affordance: "reenter_credentials",
  },
  AUTH_OAUTH_FAILED: {
    bucket: "user_correctable",
    messageKey: "err_auth_oauth_failed",
    affordance: "restart_oauth",
  },
  AUTH_KEYCHAIN_DENIED: {
    bucket: "forbidden",
    messageKey: "err_auth_keychain_denied",
    affordance: "open_keychain_guidance",
  },
  IMAP_CONNECTION_FAILED: {
    bucket: "retryable",
    messageKey: "err_imap_connection_failed",
    affordance: "retry",
  },
  IMAP_UID_VALIDITY_CHANGED: {
    bucket: "recoverable",
    messageKey: "err_imap_uid_validity_changed",
    affordance: "run_full_sync",
  },
  SMTP_SEND_FAILED: {
    bucket: "retryable",
    messageKey: "err_smtp_send_failed",
    affordance: "keep_draft",
  },
  SMTP_RATE_LIMITED: {
    bucket: "retryable",
    messageKey: "err_smtp_rate_limited",
    affordance: "auto_backoff",
  },
  DB_NOT_FOUND: {
    bucket: "internal",
    messageKey: "err_not_found",
    affordance: "refresh",
  },
  DB_CONSTRAINT: {
    bucket: "user_correctable",
    messageKey: "err_db_constraint",
    affordance: "inline",
  },
  DB_MIGRATION_FAILED: {
    bucket: "recoverable",
    messageKey: "err_db_migration_failed",
    affordance: "guided_repair",
  },
  AI_PROVIDER_UNREACHABLE: {
    bucket: "retryable",
    messageKey: "err_ai_provider_unreachable",
    affordance: "switch_provider",
  },
  AI_RATE_LIMITED: {
    bucket: "retryable",
    messageKey: "err_ai_rate_limited",
    affordance: "auto_backoff",
  },
  AI_CONTEXT_TOO_LONG: {
    bucket: "user_correctable",
    messageKey: "err_ai_context_too_long",
    affordance: "trim_context",
  },
  GTE_INDEX_CORRUPT: {
    bucket: "recoverable",
    messageKey: "err_gte_index_corrupt",
    affordance: "start_reindex",
  },
  GTE_REINDEX_IN_PROGRESS: {
    bucket: "silent",
    messageKey: "err_gte_reindex_in_progress",
    affordance: "show_progress",
  },
  FS_PERMISSION_DENIED: {
    bucket: "environment",
    messageKey: "err_fs_permission_denied",
    affordance: "os_guidance",
  },
  FS_DISK_FULL: {
    bucket: "environment",
    messageKey: "err_fs_disk_full",
    affordance: "free_space",
  },
  VALIDATION: {
    bucket: "user_correctable",
    messageKey: "err_validation",
    affordance: "inline",
  },
  NOT_FOUND: {
    bucket: "internal",
    messageKey: "err_not_found",
    affordance: "refresh",
  },
  FORBIDDEN: {
    bucket: "forbidden",
    messageKey: "err_forbidden",
    affordance: "explain",
  },
  INTERNAL: {
    bucket: "internal",
    messageKey: "err_internal",
    affordance: "report",
  },
};

/** Look up the UX for a code, defaulting to the generic INTERNAL treatment. */
export function uxForCode(code: ErrorCode): ErrorUx {
  return ERROR_UX[code] ?? ERROR_UX.INTERNAL;
}
