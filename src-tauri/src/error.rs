//! The three error surfaces and their single hand-off points (T004, 09 §1–§3).
//!
//! * Services speak [`AppError`] (`thiserror`), returned as [`AppResult`].
//! * The IPC boundary speaks [`IpcError`] (`{ code, message, detail }`, 02 §2).
//! * `From<AppError> for IpcError` is the ONLY conversion point — and the ONLY
//!   place a crossing error is logged (09 §3: "log at the boundary, not at every
//!   layer").
//!
//! [`AppError::code`] is an exhaustive match (no `_`) so a new variant cannot be
//! added without making a deliberate wire-code decision (09 §3, 09 §8).

use serde::Serialize;
use specta::Type;

use crate::types::ErrorCode;

/// Convenience alias every fallible service fn returns.
pub type AppResult<T> = Result<T, AppError>;

/// Crate-internal error enum (03 §2). Carries Rust-side context; unexpected
/// errors fold into [`AppError::Internal`] via `anyhow`.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("invalid credentials")]
    AuthInvalidCredentials,
    #[error("oauth failed: {0}")]
    AuthOAuthFailed(String),
    #[error("keychain access denied")]
    AuthKeychainDenied,
    #[error("imap connection failed: {0}")]
    ImapConnection(String),
    #[error("imap uid validity changed")]
    ImapUidValidityChanged,
    #[error("smtp send failed: {0}")]
    SmtpSend(String),
    #[error("smtp rate limited")]
    SmtpRateLimited,
    #[error("not found")]
    NotFound,
    #[error("db constraint: {0}")]
    DbConstraint(String),
    #[error("db migration failed: {0}")]
    DbMigration(String),
    #[error("ai provider unreachable: {0}")]
    AiUnreachable(String),
    #[error("ai rate limited")]
    AiRateLimited,
    #[error("ai context too long")]
    AiContextTooLong,
    #[error("gte index corrupt")]
    GteCorrupt,
    #[error("gte reindex in progress")]
    GteReindexBusy,
    #[error("filesystem permission denied: {0}")]
    FsPermission(String),
    #[error("disk full")]
    FsDiskFull,
    #[error("validation: {0}")]
    Validation(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    /// Maps every variant to its wire [`ErrorCode`]. **Exhaustive on purpose** —
    /// adding a variant without a code here is a compile error (09 §3).
    pub fn code(&self) -> ErrorCode {
        match self {
            AppError::AuthInvalidCredentials => ErrorCode::AuthInvalidCredentials,
            AppError::AuthOAuthFailed(_) => ErrorCode::AuthOauthFailed,
            AppError::AuthKeychainDenied => ErrorCode::AuthKeychainDenied,
            AppError::ImapConnection(_) => ErrorCode::ImapConnectionFailed,
            AppError::ImapUidValidityChanged => ErrorCode::ImapUidValidityChanged,
            AppError::SmtpSend(_) => ErrorCode::SmtpSendFailed,
            AppError::SmtpRateLimited => ErrorCode::SmtpRateLimited,
            AppError::NotFound => ErrorCode::NotFound,
            AppError::DbConstraint(_) => ErrorCode::DbConstraint,
            AppError::DbMigration(_) => ErrorCode::DbMigrationFailed,
            AppError::AiUnreachable(_) => ErrorCode::AiProviderUnreachable,
            AppError::AiRateLimited => ErrorCode::AiRateLimited,
            AppError::AiContextTooLong => ErrorCode::AiContextTooLong,
            AppError::GteCorrupt => ErrorCode::GteIndexCorrupt,
            AppError::GteReindexBusy => ErrorCode::GteReindexInProgress,
            AppError::FsPermission(_) => ErrorCode::FsPermissionDenied,
            AppError::FsDiskFull => ErrorCode::FsDiskFull,
            AppError::Validation(_) => ErrorCode::Validation,
            AppError::Forbidden(_) => ErrorCode::Forbidden,
            AppError::Internal(_) => ErrorCode::Internal,
        }
    }

    /// A clean, secret-free English fallback message. The frontend renders the
    /// localized copy via the `ErrorCode → i18n key` table (09 §4); this string
    /// is only a developer-facing default and a log message.
    fn user_message(&self) -> &'static str {
        match self.code() {
            ErrorCode::AuthInvalidCredentials => "We couldn't sign in with those details.",
            ErrorCode::AuthOauthFailed => "Authorization didn't complete.",
            ErrorCode::AuthKeychainDenied => "Access to your system Keychain was denied.",
            ErrorCode::ImapConnectionFailed => "Can't reach the mail server right now.",
            ErrorCode::ImapUidValidityChanged => "This mailbox needs a full resync.",
            ErrorCode::SmtpSendFailed => "The message couldn't be sent.",
            ErrorCode::SmtpRateLimited => "Sending too fast — slowing down.",
            ErrorCode::DbNotFound => "That item no longer exists.",
            ErrorCode::DbConstraint => "That conflicts with existing data.",
            ErrorCode::DbMigrationFailed => "We couldn't update the local database.",
            ErrorCode::AiProviderUnreachable => "Your AI provider isn't responding.",
            ErrorCode::AiRateLimited => "Your AI provider is rate-limiting.",
            ErrorCode::AiContextTooLong => "There's too much context for this model.",
            ErrorCode::GteIndexCorrupt => "The search index needs rebuilding.",
            ErrorCode::GteReindexInProgress => "A rebuild is already running.",
            ErrorCode::FsPermissionDenied => "We don't have permission to that location.",
            ErrorCode::FsDiskFull => "Your disk is full.",
            ErrorCode::Validation => "Please check the highlighted field.",
            ErrorCode::NotFound => "That item no longer exists.",
            ErrorCode::Forbidden => "That action isn't allowed.",
            ErrorCode::Internal => "Something went wrong on our side.",
        }
    }

    /// Technical context for logs/`detail` only — never shown raw in the UI, and
    /// never carries secrets (variant payloads hold hosts/constraint names, not
    /// passwords, addresses, or bodies; 09 §5).
    fn detail(&self) -> Option<String> {
        match self {
            AppError::AuthOAuthFailed(d)
            | AppError::ImapConnection(d)
            | AppError::SmtpSend(d)
            | AppError::DbConstraint(d)
            | AppError::DbMigration(d)
            | AppError::AiUnreachable(d)
            | AppError::FsPermission(d)
            | AppError::Validation(d)
            | AppError::Forbidden(d) => Some(d.clone()),
            AppError::Internal(e) => Some(format!("{e:#}")),
            _ => None,
        }
    }
}

/// The serialized wire error (02 §2). Tauri throws the `Err` variant of a
/// command's `Result` to JS in this shape.
#[derive(Debug, Clone, Serialize, Type)]
pub struct IpcError {
    pub code: ErrorCode,
    /// Human-readable English fallback (the UI prefers the localized key, 09 §4).
    pub message: String,
    /// Technical detail for logging; `null` when there is none. Never rendered raw.
    pub detail: Option<String>,
}

impl From<AppError> for IpcError {
    /// The single `AppError → IpcError` hand-off. Logs the failure exactly once
    /// here (09 §3) with structured, secret-free fields, then builds the payload.
    fn from(err: AppError) -> Self {
        let code = err.code();
        let detail = err.detail();
        // The one and only boundary log. Fields are identifiers/codes only — no
        // bodies, addresses, or secrets (09 §5).
        tracing::error!(
            error_code = code.as_wire(),
            detail = detail.as_deref().unwrap_or(""),
            "command failed at ipc boundary"
        );
        IpcError {
            code,
            message: err.user_message().to_string(),
            detail,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_app_error_sets_code_and_message() {
        let ipc: IpcError = AppError::Forbidden("delete primary account".into()).into();
        assert_eq!(ipc.code, ErrorCode::Forbidden);
        assert_eq!(ipc.detail.as_deref(), Some("delete primary account"));
        assert!(!ipc.message.is_empty());
    }

    #[test]
    fn keychain_denied_maps_to_forbidden_bucket_code() {
        let ipc: IpcError = AppError::AuthKeychainDenied.into();
        assert_eq!(ipc.code, ErrorCode::AuthKeychainDenied);
        assert_eq!(ipc.code.as_wire(), "AUTH_KEYCHAIN_DENIED");
        // Unit variants carry no detail.
        assert_eq!(ipc.detail, None);
    }

    #[test]
    fn internal_folds_anyhow() {
        let err = AppError::Internal(anyhow::anyhow!("boom"));
        assert_eq!(err.code(), ErrorCode::Internal);
    }
}
