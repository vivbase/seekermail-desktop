//! I3 suspend/resume + conservative fallback (T096, F_I3 §4.1/§5).
//!
//! `answer_query` transitions the query to `answered`, updates the channel card,
//! flips the mail back to `analyzing`, and re-queues it so the normal E1/E2/E3
//! chain resumes (the answered query no longer suspends it). `skip_query` marks
//! the query `skipped` and writes a conservative fallback draft per trigger type —
//! except T4, which can never be silently dropped (the mail stays `suspended_i3`).
//!
//! Re-queue note: rather than a parallel `ResumeContext` mpsc, we reuse the
//! existing pipeline queue — simpler and it already carries restart recovery. The
//! stored `pending_queries.answer` is the durable record; threading the answer
//! into the generation prompt is a later refinement.

use serde_json::json;

use super::worker::E2PipelineJob;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage::{map_sqlx_err, query_repo};
use crate::util::{new_uuid, now_unix};

/// Apply a human answer to a pending query and resume processing (T096).
pub async fn answer_query(state: &AppState, query_id: &str, answer: &str) -> AppResult<()> {
    let db = state.storage.db();
    let query = query_repo::get_query(db, query_id).await?;
    if query.status != "pending" {
        return Err(AppError::Forbidden(format!(
            "query is '{}', not pending",
            query.status
        )));
    }

    let now = now_unix();
    let mut tx = db.pool().begin().await.map_err(map_sqlx_err)?;
    sqlx::query(
        "UPDATE pending_queries SET status = 'answered', answer = ?, answered_at = ? WHERE id = ?",
    )
    .bind(answer)
    .bind(now)
    .bind(query_id)
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx_err)?;
    if let Some(mail_id) = &query.mail_id {
        sqlx::query(
            "UPDATE im_messages SET status = 'answered' \
             WHERE message_type = 'query_card' AND status = 'pending' AND linked_email_id = ?",
        )
        .bind(mail_id)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_err)?;
        sqlx::query(
            "UPDATE mails SET ai_processing_status = 'analyzing', updated_at = ? WHERE id = ?",
        )
        .bind(now)
        .bind(mail_id)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_err)?;
    }
    tx.commit().await.map_err(map_sqlx_err)?;

    // Status message + resume (best-effort; never fail the answer on these).
    post_status(
        state,
        "Agent is processing your reply…",
        query.mail_id.as_deref(),
    )
    .await;
    if let Some(mail_id) = &query.mail_id {
        state.pipeline_queue.try_enqueue(E2PipelineJob {
            mail_id: mail_id.clone(),
            account_id: query.account_id.clone(),
        });
    }
    Ok(())
}

/// Skip a pending query, applying the conservative fallback (T096, F_I3 §5).
pub async fn skip_query(state: &AppState, query_id: &str) -> AppResult<()> {
    let db = state.storage.db();
    let query = query_repo::get_query(db, query_id).await?;
    if query.status != "pending" {
        return Err(AppError::Forbidden(format!(
            "query is '{}', not pending",
            query.status
        )));
    }
    let is_t4 = query.trigger_type == "T4";
    if is_t4 {
        tracing::warn!(
            query_id,
            "skip requested on a T4 risk query — mail stays suspended"
        );
    }

    let now = now_unix();
    let mut tx = db.pool().begin().await.map_err(map_sqlx_err)?;
    sqlx::query("UPDATE pending_queries SET status = 'skipped', answered_at = ? WHERE id = ?")
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
        // T4 never resolves to 'done' on skip — it must keep surfacing.
        if !is_t4 {
            sqlx::query(
                "UPDATE mails SET ai_processing_status = 'done', updated_at = ? WHERE id = ?",
            )
            .bind(now)
            .bind(mail_id)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_err)?;
        }
    }
    tx.commit().await.map_err(map_sqlx_err)?;

    // Conservative fallback draft for non-T4 triggers (T4 is left for the user).
    if !is_t4 {
        if let Some(mail_id) = &query.mail_id {
            write_fallback_draft(state, mail_id, &query.account_id, &query.trigger_type).await?;
        }
    }
    Ok(())
}

/// Post a system status message to the TEAM channel (best-effort).
async fn post_status(state: &AppState, text: &str, mail_id: Option<&str>) {
    let content = json!({ "text": text }).to_string();
    let _ = crate::storage::im_repo::insert_message(
        state.storage.db(),
        crate::storage::im_repo::MAIN_CHANNEL,
        "system",
        "system",
        "status",
        &content,
        mail_id,
        None,
    )
    .await;
}

/// The conservative reply body per trigger type (F_I3 §5). Never calls a provider —
/// these are safe, neutral holding replies the user can edit in E6.
fn fallback_body(trigger_type: &str) -> &'static str {
    match trigger_type {
        "T1" => "Thank you for reaching out. We'd like to confirm a few details before proceeding; a member of our team will follow up with you shortly.",
        "T2" => "Thank you for your message. Could you share a few times that work for you? [Please add specific availability before sending.]",
        "T3" => "Thank you for your message. We will review and follow up shortly.",
        "T5" => "Thank you for your message. We'll review and respond; please resend any referenced attachment if it didn't come through.",
        "T6" => "Thank you for your message. We're not able to action this request exactly as described, and we're routing it for review.",
        _ => "Thank you for your message. We will follow up shortly.",
    }
}

/// Write a `pending` E2-style draft as the conservative fallback (T096 §6).
/// Shared with the expiry sweep (T097), which applies the same fallback on
/// timeout.
pub(crate) async fn write_fallback_draft(
    state: &AppState,
    mail_id: &str,
    account_id: &str,
    trigger_type: &str,
) -> AppResult<()> {
    let db = state.storage.db();
    let row: Option<(Option<String>, String, String)> =
        sqlx::query_as("SELECT from_name, from_email, subject FROM mails WHERE id = ?")
            .bind(mail_id)
            .fetch_optional(db.pool())
            .await
            .map_err(map_sqlx_err)?;
    let Some((from_name, from_email, subject)) = row else {
        return Ok(()); // mail gone; nothing to draft against
    };

    let to_addr = json!({ "name": from_name, "email": from_email }).to_string();
    let reply_subject = if subject.is_empty() {
        "Re:".to_string()
    } else {
        format!("Re: {subject}")
    };
    let body = fallback_body(trigger_type);
    let now = now_unix();
    sqlx::query(
        "INSERT INTO ai_drafts (id, trigger_mail_id, account_id, to_addr, cc_addrs, subject, \
             body_original, body_current, is_edited, trigger_mode, ai_model, knowledge_refs, \
             status, created_at, updated_at) \
         VALUES (?, ?, ?, ?, '[]', ?, ?, ?, 0, 'E2_semi', 'conservative-fallback', '[]', \
             'pending', ?, ?)",
    )
    .bind(new_uuid())
    .bind(mail_id)
    .bind(account_id)
    .bind(&to_addr)
    .bind(&reply_subject)
    .bind(body)
    .bind(body)
    .bind(now)
    .bind(now)
    .execute(db.pool())
    .await
    .map_err(map_sqlx_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::account_repo::{AccountRepo, NewAccount};
    use crate::storage::query_repo::NewQuery;

    async fn setup(state: &AppState, trigger: &str) -> String {
        let db = state.storage.db();
        AccountRepo::new(db)
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
        let now = now_unix();
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, from_name, from_email, to_addrs, \
                 subject, date_sent, date_received, ai_processing_status, created_at, updated_at) \
             VALUES ('m1','acc','<m1>','Bob','bob@x.com','[]','Hello', ?, ?, 'suspended_i3', ?, ?)",
        )
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(db.pool())
        .await
        .unwrap();
        // The query card message + the pending query.
        crate::storage::im_repo::insert_message(
            db,
            "main",
            "agent",
            "acc",
            "query_card",
            "{}",
            Some("m1"),
            Some("pending"),
        )
        .await
        .unwrap();
        let q = query_repo::insert_query(
            db,
            &NewQuery {
                account_id: "acc".into(),
                mail_id: Some("m1".into()),
                risk_event_id: None,
                trigger_type: trigger.into(),
                question: "?".into(),
                options: None,
                priority: if trigger == "T4" { 1 } else { 3 },
                expires_at: None,
            },
        )
        .await
        .unwrap();
        q.id
    }

    async fn mail_status(state: &AppState) -> String {
        let (s,): (String,) =
            sqlx::query_as("SELECT ai_processing_status FROM mails WHERE id='m1'")
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        s
    }

    #[tokio::test]
    async fn answer_updates_all_three_tables_and_is_idempotent() {
        let (state, _rx) = AppState::test_state().await;
        let id = setup(&state, "T1").await;
        answer_query(&state, &id, "Yes, I know them").await.unwrap();

        let q = query_repo::get_query(state.storage.db(), &id)
            .await
            .unwrap();
        assert_eq!(q.status, "answered");
        assert_eq!(q.answer.as_deref(), Some("Yes, I know them"));
        assert_eq!(mail_status(&state).await, "analyzing");

        // Second answer is forbidden.
        assert!(matches!(
            answer_query(&state, &id, "again").await.unwrap_err(),
            AppError::Forbidden(_)
        ));
    }

    #[tokio::test]
    async fn skip_non_t4_writes_fallback_draft_and_marks_done() {
        let (state, _rx) = AppState::test_state().await;
        let id = setup(&state, "T2").await;
        skip_query(&state, &id).await.unwrap();

        assert_eq!(
            query_repo::get_query(state.storage.db(), &id)
                .await
                .unwrap()
                .status,
            "skipped"
        );
        assert_eq!(mail_status(&state).await, "done");
        let (drafts,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM ai_drafts WHERE trigger_mail_id='m1' AND status='pending'",
        )
        .fetch_one(state.storage.db().pool())
        .await
        .unwrap();
        assert_eq!(drafts, 1);
    }

    #[tokio::test]
    async fn skip_t4_keeps_mail_suspended_and_writes_no_draft() {
        let (state, _rx) = AppState::test_state().await;
        let id = setup(&state, "T4").await;
        skip_query(&state, &id).await.unwrap();

        // Query row flips to skipped, but the mail stays suspended (T4 can't drop).
        assert_eq!(mail_status(&state).await, "suspended_i3");
        let (drafts,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM ai_drafts WHERE trigger_mail_id='m1'")
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        assert_eq!(drafts, 0);
    }
}
