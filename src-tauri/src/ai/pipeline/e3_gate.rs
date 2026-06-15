//! E3 enablement gate + whitelist filter (T085 §3, F_E3 §4.1).
//!
//! * **Gate** — full-auto requires a track record: the count of human-approved
//!   drafts (`ai_decisions.decision_type = 'draft_sent'`) must reach the
//!   account's `account_ai_settings.e3_min_history` (schema default 3; the
//!   product narrative targets 50 for GA — the per-account column is the
//!   authority, never a hardcoded constant).
//! * **Whitelist** — when `e3_whitelist_only = 1`, auto-send is limited to
//!   correspondents we have replied to at least [`WHITELIST_MIN_REPLIES`]
//!   times (`contacts.reply_count`; the schema has no dedicated whitelist
//!   table, so reply history carries the "known contact" semantic).
//!
//! Both misses demote to E2 (draft for human review) — they are filters, not
//! errors.

use crate::error::AppResult;
use crate::storage::{map_sqlx_err, Db};

/// Minimum prior replies for a sender to count as whitelisted (F_E3 §4.1).
pub const WHITELIST_MIN_REPLIES: i64 = 3;

/// Gate outcome (consumed by the pipeline and, later, the T086 progress UI).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum E3GateResult {
    Eligible,
    NotEligible { approved_so_far: i64, required: i64 },
}

/// Check the approved-draft history against the account's configured
/// threshold. `NOT_FOUND` when the account has no settings row.
pub async fn check_enabled(db: &Db, account_id: &str) -> AppResult<E3GateResult> {
    let (required,): (i64,) =
        sqlx::query_as("SELECT e3_min_history FROM account_ai_settings WHERE account_id = ?")
            .bind(account_id)
            .fetch_optional(db.pool())
            .await
            .map_err(map_sqlx_err)?
            .ok_or(crate::error::AppError::NotFound)?;
    let (approved,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM ai_decisions \
         WHERE account_id = ? AND decision_type = 'draft_sent' AND impact = 'reply'",
    )
    .bind(account_id)
    .fetch_one(db.pool())
    .await
    .map_err(map_sqlx_err)?;
    if approved >= required {
        Ok(E3GateResult::Eligible)
    } else {
        Ok(E3GateResult::NotEligible {
            approved_so_far: approved,
            required,
        })
    }
}

/// Whitelist filter: `true` when auto-send to this sender is allowed. Always
/// `true` when `e3_whitelist_only` is off.
pub async fn whitelist_allows(db: &Db, account_id: &str, from_email: &str) -> AppResult<bool> {
    let (whitelist_only,): (i64,) =
        sqlx::query_as("SELECT e3_whitelist_only FROM account_ai_settings WHERE account_id = ?")
            .bind(account_id)
            .fetch_optional(db.pool())
            .await
            .map_err(map_sqlx_err)?
            .ok_or(crate::error::AppError::NotFound)?;
    if whitelist_only == 0 {
        return Ok(true);
    }
    let reply_count: Option<(i64,)> =
        sqlx::query_as("SELECT reply_count FROM contacts WHERE email = ?")
            .bind(from_email.trim().to_lowercase())
            .fetch_optional(db.pool())
            .await
            .map_err(map_sqlx_err)?;
    Ok(reply_count.map(|(c,)| c).unwrap_or(0) >= WHITELIST_MIN_REPLIES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use crate::util::{new_uuid, now_unix};

    async fn seed_account(state: &AppState, e3_min_history: i64, whitelist_only: i64) -> String {
        let id = new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
                 created_at, updated_at) VALUES (?, ?, 'Work', 'imap', 'slate', 'W', ?, ?)",
        )
        .bind(&id)
        .bind(format!("{id}@example.com"))
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO account_ai_settings (account_id, auth_level, e3_min_history, \
                 e3_whitelist_only, updated_at) VALUES (?, 3, ?, ?, ?)",
        )
        .bind(&id)
        .bind(e3_min_history)
        .bind(whitelist_only)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        id
    }

    async fn seed_draft_sent(state: &AppState, account_id: &str, n: usize) {
        for _ in 0..n {
            sqlx::query(
                "INSERT INTO ai_decisions (id, account_id, decision_type, impact, \
                     action_description, result_description, created_at) \
                 VALUES (?, ?, 'draft_sent', 'reply', 'Approved.', 'Sent.', ?)",
            )
            .bind(new_uuid())
            .bind(account_id)
            .bind(now_unix())
            .execute(state.storage.db().pool())
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn gate_uses_the_per_account_threshold() {
        let (state, _rx) = AppState::test_state().await;
        let db = state.storage.db();
        let account = seed_account(&state, 3, 1).await;

        assert_eq!(
            check_enabled(db, &account).await.unwrap(),
            E3GateResult::NotEligible {
                approved_so_far: 0,
                required: 3
            }
        );
        seed_draft_sent(&state, &account, 3).await;
        assert_eq!(
            check_enabled(db, &account).await.unwrap(),
            E3GateResult::Eligible
        );
    }

    #[tokio::test]
    async fn whitelist_requires_reply_history() {
        let (state, _rx) = AppState::test_state().await;
        let db = state.storage.db();
        let account = seed_account(&state, 3, 1).await;

        // Unknown sender → not whitelisted.
        assert!(!whitelist_allows(db, &account, "new@vendor.example")
            .await
            .unwrap());

        let now = now_unix();
        sqlx::query(
            "INSERT INTO contacts (id, email, first_seen_at, last_seen_at, reply_count, \
                 created_at, updated_at) VALUES (?, 'known@vendor.example', ?, ?, 5, ?, ?)",
        )
        .bind(new_uuid())
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        assert!(whitelist_allows(db, &account, "known@vendor.example")
            .await
            .unwrap());

        // Whitelist off → everything passes.
        let open = seed_account(&state, 3, 0).await;
        assert!(whitelist_allows(db, &open, "new@vendor.example")
            .await
            .unwrap());
    }
}
