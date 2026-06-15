//! I3 query expiry + T4 reminders (T097, F_I3 §4.2).
//!
//! A background task scans `pending_queries` on a cadence:
//!   • non-T4 pending queries past `expires_at` (72 h) auto-expire — the same
//!     conservative fallback as a manual skip is applied (T096), the channel card
//!     flips to `skipped`, the mail finishes, and `query:expired` fires.
//!   • T4 queries never expire (they must never be silently dropped); instead one
//!     *merged* reminder per account per day is posted (the F5 pressure-relief
//!     valve — "You have N unresolved risk alerts") and `last_reminder_at` stamped.

use serde_json::json;

use crate::ai::pipeline::resume;
use crate::error::AppResult;
use crate::state::AppState;
use crate::storage::{map_sqlx_err, query_repo};
use crate::util::now_unix;

/// One full sweep: expire overdue non-T4 queries, then post due T4 reminders.
pub async fn run_query_expiry_check(state: &AppState) -> AppResult<()> {
    expire_overdue_non_t4(state).await?;
    remind_open_t4(state).await?;
    Ok(())
}

async fn expire_overdue_non_t4(state: &AppState) -> AppResult<()> {
    let db = state.storage.db();
    let now = now_unix();
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT id FROM pending_queries \
         WHERE status = 'pending' AND trigger_type != 'T4' \
           AND expires_at IS NOT NULL AND expires_at <= ?",
    )
    .bind(now)
    .fetch_all(db.pool())
    .await
    .map_err(map_sqlx_err)?;

    for (id,) in rows {
        expire_one(state, &id).await?;
    }
    Ok(())
}

/// Expire one query: identical end-state to a manual skip (T096 §6), but the row
/// status is `expired` (audit-distinct from a user skip) and `query:expired` fires.
pub async fn expire_one(state: &AppState, query_id: &str) -> AppResult<()> {
    let db = state.storage.db();
    let query = query_repo::get_query(db, query_id).await?;
    if query.status != "pending" || query.trigger_type == "T4" {
        return Ok(()); // already resolved, or a T4 (never expires)
    }

    let now = now_unix();
    let mut tx = db.pool().begin().await.map_err(map_sqlx_err)?;
    sqlx::query("UPDATE pending_queries SET status = 'expired', answered_at = ? WHERE id = ?")
        .bind(now)
        .bind(query_id)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_err)?;
    if let Some(mail_id) = &query.mail_id {
        sqlx::query(
            "UPDATE im_messages SET status = 'skipped' \
             WHERE message_type = 'query_card' AND status = 'pending' AND linked_email_id = ?",
        )
        .bind(mail_id)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_err)?;
        sqlx::query("UPDATE mails SET ai_processing_status = 'done', updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(mail_id)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_err)?;
    }
    tx.commit().await.map_err(map_sqlx_err)?;

    if let Some(mail_id) = &query.mail_id {
        resume::write_fallback_draft(state, mail_id, &query.account_id, &query.trigger_type)
            .await?;
    }
    state
        .events
        .query_expired(&query.id, &query.account_id, &query.trigger_type);
    Ok(())
}

/// Reminder cadence: at most one merged T4 reminder per account per 24 h.
const REMINDER_INTERVAL_SECS: i64 = 24 * 3600;

async fn remind_open_t4(state: &AppState) -> AppResult<()> {
    let db = state.storage.db();
    let now = now_unix();
    let due_before = now - REMINDER_INTERVAL_SECS;

    // Accounts with open T4 queries whose most-recent reminder is stale (or never).
    let accounts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT account_id, count(*) FROM pending_queries \
         WHERE status = 'pending' AND trigger_type = 'T4' \
         GROUP BY account_id \
         HAVING COALESCE(MAX(last_reminder_at), 0) <= ?",
    )
    .bind(due_before)
    .fetch_all(db.pool())
    .await
    .map_err(map_sqlx_err)?;

    for (account_id, count) in accounts {
        let text = format!(
            "You have {count} unresolved risk {} awaiting your review.",
            if count == 1 { "alert" } else { "alerts" }
        );
        let content = json!({ "text": text }).to_string();
        let _ = crate::storage::im_repo::insert_message(
            db,
            crate::storage::im_repo::MAIN_CHANNEL,
            "system",
            "system",
            "status",
            &content,
            None,
            None,
        )
        .await;
        sqlx::query(
            "UPDATE pending_queries SET last_reminder_at = ? \
             WHERE status = 'pending' AND trigger_type = 'T4' AND account_id = ?",
        )
        .bind(now)
        .bind(&account_id)
        .execute(db.pool())
        .await
        .map_err(map_sqlx_err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::account_repo::{AccountRepo, NewAccount};
    use crate::storage::query_repo::NewQuery;

    async fn seed_account(state: &AppState) {
        AccountRepo::new(state.storage.db())
            .create(&NewAccount {
                id: "acc".into(),
                email: "me@x.com".into(),
                display_name: "Me".into(),
                provider: "imap".into(),
                imap_host: None,
                imap_port: 993,
                smtp_host: None,
                smtp_port: 587,
                color_token: "slate".into(),
                badge_label: "W".into(),
                role_type: "work".into(),
                role_description: None,
                auth_level: 1,
            })
            .await
            .unwrap();
    }

    async fn seed_mail(state: &AppState, id: &str) {
        let now = now_unix();
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, from_name, from_email, to_addrs, \
                 subject, date_sent, date_received, ai_processing_status, created_at, updated_at) \
             VALUES (?, 'acc', ?, 'Bob', 'bob@x.com', '[]', 'Hi', ?, ?, 'suspended_i3', ?, ?)",
        )
        .bind(id)
        .bind(format!("<{id}>"))
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    fn query(trigger: &str, mail_id: &str, expires_at: Option<i64>) -> NewQuery {
        NewQuery {
            account_id: "acc".into(),
            mail_id: Some(mail_id.into()),
            risk_event_id: None,
            trigger_type: trigger.into(),
            question: "?".into(),
            options: None,
            priority: if trigger == "T4" { 1 } else { 3 },
            expires_at,
        }
    }

    #[tokio::test]
    async fn overdue_non_t4_expires_with_fallback_draft() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state).await;
        seed_mail(&state, "m1").await;
        let db = state.storage.db();
        // Already overdue (expires in the past).
        let q = query_repo::insert_query(db, &query("T2", "m1", Some(now_unix() - 10)))
            .await
            .unwrap();

        run_query_expiry_check(&state).await.unwrap();

        assert_eq!(
            query_repo::get_query(db, &q.id).await.unwrap().status,
            "expired"
        );
        let (status,): (String,) =
            sqlx::query_as("SELECT ai_processing_status FROM mails WHERE id='m1'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(status, "done");
        let (drafts,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM ai_drafts WHERE trigger_mail_id='m1'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(drafts, 1);
    }

    #[tokio::test]
    async fn t4_never_expires_and_gets_one_merged_reminder() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state).await;
        seed_mail(&state, "m1").await;
        seed_mail(&state, "m2").await;
        let db = state.storage.db();
        // Two open T4 queries, both "overdue" — but T4 never expires.
        query_repo::insert_query(db, &query("T4", "m1", None))
            .await
            .unwrap();
        query_repo::insert_query(db, &query("T4", "m2", None))
            .await
            .unwrap();

        run_query_expiry_check(&state).await.unwrap();

        // Both still pending.
        let pending = query_repo::list_pending(db, None).await.unwrap();
        assert_eq!(pending.iter().filter(|q| q.trigger_type == "T4").count(), 2);
        // Exactly one merged reminder message mentioning the count of 2.
        let msgs = crate::storage::im_repo::list_messages(db, None, None, None, None)
            .await
            .unwrap();
        let reminders: Vec<_> = msgs
            .items
            .iter()
            .filter(|m| m.message_type == "status" && m.content.contains("unresolved risk"))
            .collect();
        assert_eq!(reminders.len(), 1);
        assert!(reminders[0].content.contains('2'));

        // A second immediate sweep does not post another reminder (24 h gate).
        run_query_expiry_check(&state).await.unwrap();
        let msgs2 = crate::storage::im_repo::list_messages(db, None, None, None, None)
            .await
            .unwrap();
        let reminders2 = msgs2
            .items
            .iter()
            .filter(|m| m.message_type == "status" && m.content.contains("unresolved risk"))
            .count();
        assert_eq!(reminders2, 1);
    }
}
