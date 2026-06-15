//! I3 proactive-query stage (T095, F_I3 §3). Runs after E4 pre-scan and before
//! the E1/E2/E3 chain: it detects T1–T6 triggers for one freshly ingested mail,
//! and if any fire it raises a structured query — writing `pending_queries`, a
//! `query_card` message to the TEAM channel, and marking the mail `suspended_i3`
//! so the auth-route chain pauses until the user answers (T096).
//!
//! Anti-over-notify (F_I3 §6): a mail already carrying a query is skipped, the
//! same sender/trigger is deduplicated within 48 h, and once an account is over
//! its daily cap new single cards are suppressed (the consolidated day-end
//! summary is produced by the T097 scheduler, a better home for the dedup state).

use crate::ai::query_detection::{
    detect_query_triggers, DetectionInput, QueryTrigger, TriggerFlags,
};
use crate::error::AppResult;
use crate::state::AppState;
use crate::storage::query_repo::{NewQuery, DAILY_CARD_CAP, QUERY_TTL_SECS};
use crate::storage::{map_sqlx_err, query_repo};
use crate::util::now_unix;

/// Outcome of [`run_i3_detection`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum I3Outcome {
    /// A query was raised; the caller must NOT run the E1/E2/E3 chain now.
    Suspended { query_id: String },
    /// No trigger — the caller proceeds with normal AI processing.
    Clear,
}

/// The i3-relevant mail fields (a superset of `PipelineMail` — it also needs the
/// reply chain and attachment flag).
struct I3Mail {
    from_email: String,
    body: String,
    has_attachments: bool,
    has_reply_context: bool,
}

/// DB projection for [`load_i3_mail`] (a named struct rather than a wide tuple).
#[derive(sqlx::FromRow)]
struct I3MailRow {
    from_email: String,
    body_text: Option<String>,
    snippet: Option<String>,
    has_attachments: i64,
    in_reply_to: Option<String>,
}

/// Run I3 detection for one mail. Returns [`I3Outcome::Clear`] for any account
/// without AI settings (nothing to gate) or with no triggers.
pub async fn run_i3_detection(
    state: &AppState,
    mail_id: &str,
    account_id: &str,
) -> AppResult<I3Outcome> {
    let db = state.storage.db();

    // Skip if this mail already carries a query (idempotent re-ingest, F_I3 §3.3).
    if query_repo::mail_has_active_query(db, mail_id).await? {
        return Ok(I3Outcome::Clear);
    }

    let Some(mail) = load_i3_mail(state, mail_id).await? else {
        return Ok(I3Outcome::Clear);
    };
    let Some(flags) = load_trigger_flags(state, account_id).await? else {
        // No account_ai_settings row → AI not configured → skip quietly.
        return Ok(I3Outcome::Clear);
    };

    let is_new_sender = is_new_sender(state, &mail.from_email).await?;
    let t4_risk_event_id = open_t4_risk_event(state, mail_id).await?;

    let triggers = detect_query_triggers(
        &DetectionInput {
            body: mail.body.clone(),
            is_new_sender,
            has_attachments: mail.has_attachments,
            has_reply_context: mail.has_reply_context,
            t4_risk_event_id,
        },
        &flags,
    );
    let Some(primary) = pick_primary(&triggers) else {
        return Ok(I3Outcome::Clear);
    };

    // Dedup: same sender + trigger within 48 h (T4 is never deduplicated).
    if primary.trigger_type != "T4"
        && query_repo::sender_has_recent_query(
            db,
            account_id,
            &mail.from_email,
            &primary.trigger_type,
        )
        .await?
    {
        return Ok(I3Outcome::Clear);
    }
    // Daily cap (T4 exempt — risk alerts are never suppressed).
    if primary.trigger_type != "T4"
        && query_repo::daily_count(db, account_id).await? >= DAILY_CARD_CAP
    {
        return Ok(I3Outcome::Clear);
    }

    let expires_at = if triggers.iter().any(|t| t.trigger_type == "T4") {
        None // any T4 in the set → never expires
    } else {
        Some(now_unix() + QUERY_TTL_SECS)
    };
    let question = question_text(&primary.trigger_type);

    let query = query_repo::insert_query(
        db,
        &NewQuery {
            account_id: account_id.to_string(),
            mail_id: Some(mail_id.to_string()),
            risk_event_id: primary.risk_event_id.clone(),
            trigger_type: primary.trigger_type.clone(),
            question: question.to_string(),
            options: None,
            priority: primary.priority,
            expires_at,
        },
    )
    .await?;

    // Build the full QA card (T098); store it on the query row (so the Pending
    // DecisionCard is self-contained) AND post it to the shared channel.
    let card = crate::ai::qa_card::generate_qa_card_content(
        &primary.trigger_type,
        primary.priority,
        &query.id,
        mail_id,
        question,
    );
    if let Err(e) = crate::ai::qa_card::validate_qa_card_content(&card) {
        // Defensive: a malformed card is logged, never fatal to ingestion.
        tracing::warn!(error = %e.0, trigger = %primary.trigger_type, "qa card validation failed");
    }
    let content = serde_json::to_string(&card).unwrap_or_else(|_| "{}".to_string());
    query_repo::set_options(db, &query.id, &content).await?;
    crate::storage::im_repo::insert_message(
        db,
        crate::storage::im_repo::MAIN_CHANNEL,
        "agent",
        account_id,
        "query_card",
        &content,
        Some(mail_id),
        Some("pending"),
    )
    .await?;

    set_mail_status(state, mail_id, "suspended_i3").await?;

    // Notify the UI. T4 also re-raises its risk alert so the banner shows live.
    let priority = if primary.priority <= 1 {
        "high"
    } else {
        "normal"
    };
    state.events.query_new(&query.id, account_id, priority);
    if let Some(risk_id) = &primary.risk_event_id {
        state.events.risk_alert(risk_id, mail_id, account_id);
    }

    Ok(I3Outcome::Suspended { query_id: query.id })
}

/// The highest-priority trigger (lowest `priority` number); ties keep detection
/// order (T4 first).
fn pick_primary(triggers: &[QueryTrigger]) -> Option<QueryTrigger> {
    triggers.iter().min_by_key(|t| t.priority).cloned()
}

fn question_text(trigger_type: &str) -> &'static str {
    match trigger_type {
        "T1" => "An unknown sender raised a sensitive topic. Do you recognise them?",
        "T2" => "This message asks to meet but gives no time. How should the agent reply?",
        "T3" => "There are several equally valid ways to handle this. Which do you prefer?",
        "T4" => "A high-risk item was flagged. Confirm how the agent should proceed.",
        "T5" => "This message references something that isn't here. What should the agent do?",
        "T6" => "This request may cross the agent's configured boundaries. How should it proceed?",
        _ => "The agent needs your input on this message.",
    }
}

// ── DB helpers ──────────────────────────────────────────────────────────────

async fn load_i3_mail(state: &AppState, mail_id: &str) -> AppResult<Option<I3Mail>> {
    let row: Option<I3MailRow> = sqlx::query_as(
        "SELECT from_email, body_text, snippet, has_attachments, in_reply_to \
         FROM mails WHERE id = ? AND is_deleted = 0",
    )
    .bind(mail_id)
    .fetch_optional(state.storage.db().pool())
    .await
    .map_err(map_sqlx_err)?;
    Ok(row.map(|r| I3Mail {
        from_email: r.from_email,
        body: r.body_text.or(r.snippet).unwrap_or_default(),
        has_attachments: r.has_attachments != 0,
        has_reply_context: r.in_reply_to.is_some(),
    }))
}

async fn load_trigger_flags(state: &AppState, account_id: &str) -> AppResult<Option<TriggerFlags>> {
    let row: Option<(i64, i64, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT t1_enabled, t2_enabled, t3_enabled, t4_enabled, t5_enabled, t6_enabled \
         FROM account_ai_settings WHERE account_id = ?",
    )
    .bind(account_id)
    .fetch_optional(state.storage.db().pool())
    .await
    .map_err(map_sqlx_err)?;
    Ok(row.map(|(t1, t2, t3, t4, t5, t6)| TriggerFlags {
        t1: t1 != 0,
        t2: t2 != 0,
        t3: t3 != 0,
        t4: t4 != 0,
        t5: t5 != 0,
        t6: t6 != 0,
    }))
}

/// A sender with no `contacts` row, or `interaction_count == 0`, is "new".
async fn is_new_sender(state: &AppState, from_email: &str) -> AppResult<bool> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT interaction_count FROM contacts WHERE email = ?")
            .bind(from_email)
            .fetch_optional(state.storage.db().pool())
            .await
            .map_err(map_sqlx_err)?;
    Ok(row.map(|(c,)| c == 0).unwrap_or(true))
}

/// The id of an open level-4 risk event for this mail (E4 → T4 bridge).
async fn open_t4_risk_event(state: &AppState, mail_id: &str) -> AppResult<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM risk_events \
         WHERE mail_id = ? AND risk_level = 4 AND status = 'open' \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(mail_id)
    .fetch_optional(state.storage.db().pool())
    .await
    .map_err(map_sqlx_err)?;
    Ok(row.map(|(id,)| id))
}

async fn set_mail_status(state: &AppState, mail_id: &str, status: &str) -> AppResult<()> {
    sqlx::query("UPDATE mails SET ai_processing_status = ?, updated_at = ? WHERE id = ?")
        .bind(status)
        .bind(now_unix())
        .bind(mail_id)
        .execute(state.storage.db().pool())
        .await
        .map_err(map_sqlx_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::account_repo::{AccountRepo, NewAccount};

    async fn seed(state: &AppState, mail_id: &str, from_email: &str, body: &str, attach: bool) {
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
            "INSERT INTO mails (id, account_id, message_id, from_email, to_addrs, body_text, \
                 date_sent, date_received, has_attachments, created_at, updated_at) \
             VALUES (?, 'acc', ?, ?, '[]', ?, ?, ?, ?, ?, ?)",
        )
        .bind(mail_id)
        .bind(format!("<{mail_id}>"))
        .bind(from_email)
        .bind(body)
        .bind(now)
        .bind(now)
        .bind(if attach { 1 } else { 0 })
        .bind(now)
        .bind(now)
        .execute(db.pool())
        .await
        .unwrap();
    }

    async fn mail_status(state: &AppState, mail_id: &str) -> String {
        let (s,): (String,) = sqlx::query_as("SELECT ai_processing_status FROM mails WHERE id = ?")
            .bind(mail_id)
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        s
    }

    #[tokio::test]
    async fn t1_new_sender_suspends_and_writes_card() {
        let (state, _rx) = AppState::test_state().await;
        // New sender (no contacts row) + risk keyword + attachment present (no T5).
        seed(
            &state,
            "m1",
            "stranger@x.com",
            "Please sign the attached contract today.",
            true,
        )
        .await;

        let outcome = run_i3_detection(&state, "m1", "acc").await.unwrap();
        assert!(matches!(outcome, I3Outcome::Suspended { .. }));
        assert_eq!(mail_status(&state, "m1").await, "suspended_i3");

        // A pending query + a query_card message exist.
        assert_eq!(
            query_repo::count_pending(state.storage.db()).await.unwrap(),
            1
        );
        let msgs =
            crate::storage::im_repo::list_messages(state.storage.db(), None, None, None, None)
                .await
                .unwrap();
        assert!(msgs
            .items
            .iter()
            .any(|m| m.message_type == "query_card" && m.status == "pending"));
    }

    #[tokio::test]
    async fn no_trigger_is_clear() {
        let (state, _rx) = AppState::test_state().await;
        // Known sender (seed a contact) + benign body.
        seed(&state, "m1", "friend@x.com", "Thanks, talk soon!", false).await;
        sqlx::query(
            "INSERT INTO contacts (id, email, first_seen_at, last_seen_at, interaction_count, \
                 created_at, updated_at) VALUES ('c1','friend@x.com',0,0,5,0,0)",
        )
        .execute(state.storage.db().pool())
        .await
        .unwrap();

        let outcome = run_i3_detection(&state, "m1", "acc").await.unwrap();
        assert_eq!(outcome, I3Outcome::Clear);
        assert_eq!(mail_status(&state, "m1").await, "none");
    }

    #[tokio::test]
    async fn second_pass_on_same_mail_is_clear() {
        let (state, _rx) = AppState::test_state().await;
        seed(
            &state,
            "m1",
            "stranger@x.com",
            "Please sign the attached contract.",
            true,
        )
        .await;
        assert!(matches!(
            run_i3_detection(&state, "m1", "acc").await.unwrap(),
            I3Outcome::Suspended { .. }
        ));
        // Re-running must not create a second card (idempotent).
        assert_eq!(
            run_i3_detection(&state, "m1", "acc").await.unwrap(),
            I3Outcome::Clear
        );
        assert_eq!(
            query_repo::count_pending(state.storage.db()).await.unwrap(),
            1
        );
    }
}
