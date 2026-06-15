//! Small cross-cutting helpers shared by the Module A/B cards (T013–T029).

use uuid::Uuid;

use crate::error::{AppError, AppResult};

/// Current wall-clock time as a Unix timestamp (seconds). The single source of
/// "now" so every row's `created_at`/`updated_at` is consistent and mockable.
pub fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// A fresh v4 UUID as a lowercase hyphenated string (the form stored in every
/// `TEXT … PRIMARY KEY` id column, 01).
pub fn new_uuid() -> String {
    Uuid::new_v4().to_string()
}

/// Parse a wire id string into a [`Uuid`], mapping a malformed value to a
/// `VALIDATION` error rather than a panic.
pub fn parse_uuid(s: &str) -> AppResult<Uuid> {
    Uuid::parse_str(s).map_err(|_| AppError::Validation(format!("invalid uuid: {s}")))
}

/// Normalise an email address for storage and de-duplication: trim + lowercase.
pub fn normalize_email(raw: &str) -> String {
    raw.trim().to_lowercase()
}

/// Truncate a string to at most `max` Unicode scalar values without splitting a
/// `char` (used for snippets, 01 `mails.snippet`).
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_email_trims_and_lowercases() {
        assert_eq!(normalize_email("  USER@Gmail.COM "), "user@gmail.com");
    }

    #[test]
    fn truncate_is_char_safe() {
        // 4 multi-byte chars; truncating to 2 must not panic or split bytes.
        assert_eq!(truncate_chars("héllo", 2), "hé");
        assert_eq!(truncate_chars("ok", 5), "ok");
    }

    #[test]
    fn parse_uuid_rejects_garbage() {
        assert!(parse_uuid("not-a-uuid").is_err());
        assert!(parse_uuid("00000000-0000-0000-0000-000000000000").is_ok());
    }
}
