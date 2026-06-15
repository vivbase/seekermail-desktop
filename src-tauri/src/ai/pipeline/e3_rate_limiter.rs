//! E3 auto-send rate limits (T085 §3, F_E3 §4.4).
//!
//! * **Daily cap** — at most [`E3_DAILY_LIMIT`] `auto_reply_sent` decisions
//!   per account per rolling 24 h window. (The spec says "since local
//!   midnight"; a rolling `now - 86400` window is used instead because the
//!   audit rows carry UTC unix stamps and a local-midnight computation would
//!   need timezone state — the rolling window is strictly no more permissive.)
//! * **Per-recipient cap** — at most [`E3_RECIPIENT_24H_LIMIT`] auto-replies
//!   to the same correspondent per 24 h. Recipient attribution goes through
//!   the trigger mail: the decision's `mail_id` joins `mails.from_email`
//!   (the auto-reply's recipient is the trigger mail's sender).
//!
//! A breach demotes to E2 — the draft waits for a human, nothing is dropped.

use crate::error::AppResult;
use crate::storage::{map_sqlx_err, Db};

/// Daily auto-send cap per account (F_E3 §4.4).
pub const E3_DAILY_LIMIT: i64 = 50;
/// Auto-replies to one correspondent per rolling 24 h (F_E3 §4.4).
pub const E3_RECIPIENT_24H_LIMIT: i64 = 3;

const DAY_SECS: i64 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitResult {
    Ok,
    DailyLimitReached,
    RecipientLimitReached,
}

/// Check both caps for one prospective auto-send. `recipient_email` is the
/// trigger mail's sender (= the reply's recipient). `now` is injected for
/// testability.
pub async fn check_rate_limits(
    db: &Db,
    account_id: &str,
    recipient_email: &str,
    now: i64,
) -> AppResult<RateLimitResult> {
    let (daily,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM ai_decisions \
         WHERE account_id = ? AND decision_type = 'auto_reply_sent' AND created_at > ?",
    )
    .bind(account_id)
    .bind(now - DAY_SECS)
    .fetch_one(db.pool())
    .await
    .map_err(map_sqlx_err)?;
    if daily >= E3_DAILY_LIMIT {
        tracing::warn!(
            event = "rate_limit_reached",
            account_id = %account_id,
            scope = "daily",
            count = daily,
            "e3 daily auto-send limit reached; demoting to e2"
        );
        return Ok(RateLimitResult::DailyLimitReached);
    }

    let (per_recipient,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM ai_decisions \
         WHERE account_id = ? AND decision_type = 'auto_reply_sent' AND created_at > ? \
           AND mail_id IN (SELECT id FROM mails WHERE from_email = ? AND account_id = ?)",
    )
    .bind(account_id)
    .bind(now - DAY_SECS)
    .bind(recipient_email.trim().to_lowercase())
    .bind(account_id)
    .fetch_one(db.pool())
    .await
    .map_err(map_sqlx_err)?;
    if per_recipient >= E3_RECIPIENT_24H_LIMIT {
        tracing::warn!(
            event = "rate_limit_reached",
            account_id = %account_id,
            scope = "recipient",
            count = per_recipient,
            "e3 per-recipient auto-send limit reached; demoting to e2"
        );
        return Ok(RateLimitResult::RecipientLimitReached);
    }
    Ok(RateLimitResult::Ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use crate::util::{new_uuid, now_unix};

    async fn seed_account(state: &AppState) -> String {
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
        id
    }

    async fn seed_mail(state: &AppState, id: &str, account_id: &str, from_email: &str) {
        let now = now_unix();
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, subject, from_email, to_addrs, \
                 date_sent, date_received, created_at, updated_at) \
             VALUES (?, ?, ?, 'S', ?, '[]', ?, ?, 0, 0)",
        )
        .bind(id)
        .bind(account_id)
        .bind(format!("<{id}@x>"))
        .bind(from_email)
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    async fn seed_auto_sent(state: &AppState, account_id: &str, mail_id: Option<&str>, at: i64) {
        sqlx::query(
            "INSERT INTO ai_decisions (id, account_id, mail_id, decision_type, impact, \
                 action_description, result_description, created_at) \
             VALUES (?, ?, ?, 'auto_reply_sent', 'reply', 'Auto sent.', 'Sent.', ?)",
        )
        .bind(new_uuid())
        .bind(account_id)
        .bind(mail_id)
        .bind(at)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn daily_cap_trips_at_fifty() {
        let (state, _rx) = AppState::test_state().await;
        let db = state.storage.db();
        let account = seed_account(&state).await;
        let now = now_unix();
        for _ in 0..49 {
            seed_auto_sent(&state, &account, None, now - 100).await;
        }
        assert_eq!(
            check_rate_limits(db, &account, "x@y.z", now).await.unwrap(),
            RateLimitResult::Ok
        );
        seed_auto_sent(&state, &account, None, now - 100).await;
        assert_eq!(
            check_rate_limits(db, &account, "x@y.z", now).await.unwrap(),
            RateLimitResult::DailyLimitReached
        );
        // Entries older than 24 h roll off.
        assert_eq!(
            check_rate_limits(db, &account, "x@y.z", now + 2 * 86_400)
                .await
                .unwrap(),
            RateLimitResult::Ok
        );
    }

    #[tokio::test]
    async fn recipient_cap_trips_at_three() {
        let (state, _rx) = AppState::test_state().await;
        let db = state.storage.db();
        let account = seed_account(&state).await;
        let now = now_unix();
        for i in 0..3 {
            let mail_id = format!("m{i}");
            seed_mail(&state, &mail_id, &account, "daniel@vendorco.example").await;
            seed_auto_sent(&state, &account, Some(&mail_id), now - 100).await;
        }
        assert_eq!(
            check_rate_limits(db, &account, "daniel@vendorco.example", now)
                .await
                .unwrap(),
            RateLimitResult::RecipientLimitReached
        );
        // Other recipients are unaffected.
        assert_eq!(
            check_rate_limits(db, &account, "ana@other.example", now)
                .await
                .unwrap(),
            RateLimitResult::Ok
        );
    }
}
