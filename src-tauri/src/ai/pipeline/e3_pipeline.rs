//! E3 full-auto pipeline (T085 §3, F_E3 §4, AI_MODES §4.3/§8.2).
//!
//! Sequence per mail: auth gate (`Full`) → kill switch → E4 pre-scan → E3
//! enablement gate → whitelist → loop detection → rate limits → needs-reply →
//! generation (shared path, `trigger_mode = 'E3_auto'`) → six-point self-check
//! → 30 s delayed send queue.
//!
//! Every guard that fails *demotes toward more human control* (dev/06 §7):
//! pre-generation misses run the mail through the E2 semi-auto path instead
//! (draft for review), post-generation check failures keep the generated
//! draft pending with `send_after = NULL` plus a `downgrade_e3_to_e2` audit
//! row. Nothing is ever silently dropped or sent blind.
//!
//! Kill switch: `app_settings['ai.e3_paused_until']` (written by the T086 UI)
//! holds a unix-seconds deadline as a raw integer string; `"0"` or absent =
//! not paused. A JSON-number string is tolerated too.

use crate::ai::draft::prompt_builder::TriggerMode;
use crate::ai::settings::{resolve_auth_route, AuthRouteDecision};
use crate::ai::style::StyleProfileJson;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage::{map_sqlx_err, SettingRepo};
use crate::types::AiDraft;
use crate::util::now_unix;

use super::e3_checker::{self, E3_BLOCKED_TERMS_KEY};
use super::e3_gate::{self, E3GateResult};
use super::e3_rate_limiter::{self, RateLimitResult};
use super::e3_send_queue::{self, E3_SEND_DELAY_SECS};
use super::e4_router::MailRouteDecision;
use super::{account_email, load_mail, needs_reply, PipelineMail};

/// `app_settings` key for the E3 kill switch (T086 writes it; raw integer
/// string, e.g. `"1765432100"`; `"0"`/absent = not paused).
pub const E3_PAUSED_UNTIL_KEY: &str = "ai.e3_paused_until";

/// Auto-reply chain length at which a thread is declared a mail loop
/// (F_E3 §6).
pub const LOOP_CHAIN_MAX: i64 = 4;

/// Outcome of one E3 pipeline run (T085 §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum E3Outcome {
    /// Not a Full-auto account / no reply needed / mail vanished.
    Skipped,
    /// E4 routed the mail to Trash.
    Trashed,
    /// E4 intercepted the mail into the forced-draft path.
    SensitiveDraft,
    /// A guard demoted the mail to E2 review (`draft_id` when one exists).
    Demoted { draft_id: Option<String> },
    /// Loop detection stopped the thread's auto-replies.
    Discarded,
    /// All checks passed; the draft sits in the 30 s send queue.
    Queued { draft_id: String },
}

/// Is the E3 kill switch engaged at `now`? Tolerates a raw integer string,
/// a JSON number, or a quoted number; anything unparseable = not paused.
pub async fn e3_paused(state: &AppState, now: i64) -> AppResult<bool> {
    let raw = SettingRepo::new(state.storage.db())
        .get(E3_PAUSED_UNTIL_KEY)
        .await?;
    let Some(raw) = raw else {
        return Ok(false);
    };
    let trimmed = raw.trim().trim_matches('"');
    let until = trimmed.parse::<i64>().unwrap_or(0);
    Ok(until > now)
}

/// Auto-reply chain length for a thread (F_E3 §6): own sent mails plus mails
/// from automated senders. Four or more means the thread is looping.
pub async fn chain_length(state: &AppState, thread_id: &str) -> AppResult<i64> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM mails \
         WHERE thread_id = ? AND (is_sent = 1 \
             OR from_email LIKE '%noreply%' OR from_email LIKE '%no-reply%' \
             OR from_email LIKE '%auto%' OR from_email LIKE '%bot%')",
    )
    .bind(thread_id)
    .fetch_one(state.storage.db().pool())
    .await
    .map_err(map_sqlx_err)?;
    Ok(count)
}

/// Write the `downgrade_e3_to_e2` audit row for an existing draft and notify
/// the Pending queue (`draft:ready`). The draft itself must already be
/// `pending` with `send_after = NULL` — this records and broadcasts the
/// demotion, it does not mutate the row. Shared with the send queue's
/// SMTP-failure and kill-switch paths.
pub(super) async fn record_demotion(
    state: &AppState,
    draft: &AiDraft,
    reason: &str,
) -> AppResult<()> {
    state
        .audit
        .log_await(crate::ai::audit::AuditEntry {
            account_id: draft.account_id.clone(),
            mail_id: Some(draft.trigger_mail_id.clone()),
            draft_id: Some(draft.id.clone()),
            decision_type: crate::ai::audit::decision_type::DOWNGRADE_E3_TO_E2.to_string(),
            impact: "reply".into(),
            action_description: format!("E3 auto-send demoted to E2 review ({reason})."),
            result_description: "Draft kept pending for human review; nothing was sent.".into(),
            knowledge_refs: Vec::new(),
            knowledge_summary: None,
            ai_model: Some(draft.ai_model.clone()),
            input_tokens: None,
            output_tokens: None,
            latency_ms: None,
        })
        .await?;
    state.events.draft_ready(
        &draft.id,
        &draft.trigger_mail_id,
        &draft.trigger_mode,
        &draft.account_id,
    );
    tracing::info!(
        event = "downgrade_e3_to_e2",
        draft_id = %draft.id,
        account_id = %draft.account_id,
        reason = reason,
        "e3 draft demoted to e2 review"
    );
    Ok(())
}

/// Pre-generation demotion: run the mail through the E2 semi-auto flow
/// (needs-reply + generation for review) and audit the downgrade. Returns the
/// review draft when one was generated.
async fn demote_before_generation(
    state: &AppState,
    mail: &PipelineMail,
    own_email: &str,
    reason: &str,
) -> AppResult<E3Outcome> {
    if !needs_reply::needs_reply(state, mail, own_email).await? {
        tracing::debug!(
            event = "e3_demote_no_reply",
            mail_id = %mail.id,
            reason = reason,
            "demoted mail needs no reply; nothing generated"
        );
        return Ok(E3Outcome::Demoted { draft_id: None });
    }
    let instruction = super::resume::answer_instruction_for_mail(state, &mail.id).await;
    let draft = crate::ai::draft::engine::generate_and_store(
        state,
        &mail.id,
        TriggerMode::E2Semi,
        instruction.as_deref(),
    )
    .await?;
    // generate_and_store already emitted draft:ready; record the downgrade
    // without a second event.
    state
        .audit
        .log_await(crate::ai::audit::AuditEntry {
            account_id: draft.account_id.clone(),
            mail_id: Some(draft.trigger_mail_id.clone()),
            draft_id: Some(draft.id.clone()),
            decision_type: crate::ai::audit::decision_type::DOWNGRADE_E3_TO_E2.to_string(),
            impact: "reply".into(),
            action_description: format!("E3 auto-send demoted to E2 review ({reason})."),
            result_description: "Draft generated for human review instead of auto-send.".into(),
            knowledge_refs: Vec::new(),
            knowledge_summary: None,
            ai_model: Some(draft.ai_model.clone()),
            input_tokens: None,
            output_tokens: None,
            latency_ms: None,
        })
        .await?;
    tracing::info!(
        event = "downgrade_e3_to_e2",
        draft_id = %draft.id,
        account_id = %draft.account_id,
        reason = reason,
        "e3 pipeline demoted pre-generation; e2 review draft created"
    );
    Ok(E3Outcome::Demoted {
        draft_id: Some(draft.id),
    })
}

/// Run the E3 full-auto pipeline for one ingested mail.
pub async fn run_e3_for_mail(
    state: &AppState,
    mail_id: &str,
    account_id: &str,
) -> AppResult<E3Outcome> {
    let db = state.storage.db();

    // 1) Authorization gate: E3 serves Full-auto accounts only.
    match resolve_auth_route(db, account_id).await {
        Ok(AuthRouteDecision::Full) => {}
        Ok(_) => return Ok(E3Outcome::Skipped),
        Err(AppError::NotFound) => return Ok(E3Outcome::Skipped),
        Err(e) => return Err(e),
    }

    let now = now_unix();

    // Concurrency permits — shared with E2 (global 4 / per-account 2).
    let _global = state
        .e2_semaphore
        .acquire()
        .await
        .map_err(|_| AppError::Internal(anyhow::anyhow!("pipeline semaphore closed")))?;
    let account_sem = state.e2_account_sem(account_id);
    let _local = account_sem
        .acquire_owned()
        .await
        .map_err(|_| AppError::Internal(anyhow::anyhow!("pipeline account semaphore closed")))?;

    // 2) Mail snapshot.
    let Some(mail) = load_mail(db, mail_id).await? else {
        return Ok(E3Outcome::Skipped);
    };
    if mail.is_sent != 0 {
        return Ok(E3Outcome::Skipped);
    }
    // Idempotency (same query as E2): one live draft per trigger mail.
    if super::e2_pipeline::has_live_draft(state, mail_id).await? {
        return Ok(E3Outcome::Skipped);
    }
    let own_email = account_email(db, account_id).await?.unwrap_or_default();

    // 3) Kill switch: while paused the account behaves as Semi (T086 / T085
    // §3 — demote, never skip).
    if e3_paused(state, now).await? {
        return demote_before_generation(state, &mail, &own_email, "e3_paused").await;
    }

    // 4) E4 pre-scan checkpoint (T084) — shared with the E2 pipeline.
    match super::e2_pipeline::e4_checkpoint(state, &mail).await? {
        MailRouteDecision::Proceed => {}
        MailRouteDecision::Trashed => return Ok(E3Outcome::Trashed),
        MailRouteDecision::SensitiveDraft => return Ok(E3Outcome::SensitiveDraft),
    }

    // 5) Enablement gate: enough human-approved history (per-account value).
    if let E3GateResult::NotEligible {
        approved_so_far,
        required,
    } = e3_gate::check_enabled(db, account_id).await?
    {
        tracing::info!(
            event = "e3_gate_not_eligible",
            account_id = %account_id,
            approved_so_far = approved_so_far,
            required = required,
            "e3 gate not met; demoting to e2"
        );
        return demote_before_generation(state, &mail, &own_email, "gate_not_met").await;
    }

    // 6) Whitelist filter.
    if !e3_gate::whitelist_allows(db, account_id, &mail.from_email).await? {
        return demote_before_generation(state, &mail, &own_email, "not_whitelisted").await;
    }

    // 7) Loop detection: stop the thread's auto-replies entirely.
    if let Some(thread_id) = mail.thread_id.as_deref() {
        if chain_length(state, thread_id).await? >= LOOP_CHAIN_MAX {
            // Discard any still-active drafts in the looping thread.
            sqlx::query(
                "UPDATE ai_drafts SET status = 'discarded', discard_reason = 'loop_detected', \
                     discarded_at = ?, updated_at = ? \
                 WHERE status IN ('pending', 'edited') \
                   AND trigger_mail_id IN (SELECT id FROM mails WHERE thread_id = ?)",
            )
            .bind(now)
            .bind(now)
            .bind(thread_id)
            .execute(db.pool())
            .await
            .map_err(map_sqlx_err)?;
            state.events.auto_loop_detected(thread_id, account_id);
            tracing::warn!(
                event = "auto_loop_detected",
                thread_id = %thread_id,
                account_id = %account_id,
                "mail loop detected; auto-replies stopped for this thread"
            );
            return Ok(E3Outcome::Discarded);
        }
    }

    // 8) Rate limits (daily + per-recipient).
    if e3_rate_limiter::check_rate_limits(db, account_id, &mail.from_email, now).await?
        != RateLimitResult::Ok
    {
        return demote_before_generation(state, &mail, &own_email, "rate_limited").await;
    }

    // 9) Needs-reply decision (shared with E2, thread-cached).
    if !needs_reply::needs_reply(state, &mail, &own_email).await? {
        return Ok(E3Outcome::Skipped);
    }

    // 10) Generate through the shared path (E3_auto mode). On a resumed mail,
    // fold the operator's answer to the proactive query back in (T096).
    let instruction = super::resume::answer_instruction_for_mail(state, mail_id).await;
    let draft = crate::ai::draft::engine::generate_and_store(
        state,
        mail_id,
        TriggerMode::E3Auto,
        instruction.as_deref(),
    )
    .await?;

    // 11) Six-point self-check (pure).
    let blocked_terms: Vec<String> = SettingRepo::new(db)
        .get(E3_BLOCKED_TERMS_KEY)
        .await?
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default();
    let style: Option<StyleProfileJson> = crate::ai::style::load_style_profile(db, account_id)
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok());
    let violations = e3_checker::check_draft(
        &draft.body_current,
        draft.cc_addrs.len(),
        &mail,
        style.as_ref(),
        &blocked_terms,
    );
    if !violations.is_empty() {
        let names: Vec<&str> = violations.iter().map(|v| v.as_str()).collect();
        // The draft is already pending with send_after = NULL — record the
        // demotion (audit + draft:ready so the queue surfaces it as review).
        record_demotion(state, &draft, &format!("self_check:{}", names.join("+"))).await?;
        return Ok(E3Outcome::Demoted {
            draft_id: Some(draft.id),
        });
    }

    // 12) All clear → the 30 s undo window starts now.
    e3_send_queue::enqueue(state, &draft.id, E3_SEND_DELAY_SECS).await?;
    Ok(E3Outcome::Queued { draft_id: draft.id })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ai::mock::MockProvider;
    use crate::ai::provider::ProviderError;
    use crate::ai::types::{ChatResponse, FinishReason, TokenUsage};
    use crate::types::AiProvider;
    use crate::util::new_uuid;

    async fn seed_account(state: &AppState, auth_level: i64) -> String {
        let id = new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, smtp_host, smtp_port, \
                 color_token, badge_label, role_type, role_description, auth_level, \
                 created_at, updated_at) \
             VALUES (?, 'me@example.com', 'Maya Chen', 'imap', 'smtp.example.com', 587, \
                 'slate', 'W', 'work', 'Coordinate vendor contracts.', ?, ?, ?)",
        )
        .bind(&id)
        .bind(auth_level)
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        // Whitelist off + zero history gate so the happy path runs (the gate
        // has its own dedicated test).
        sqlx::query(
            "INSERT INTO account_ai_settings (account_id, auth_level, ai_provider, ai_model, \
                 daily_query_limit, e3_whitelist_only, e3_min_history, updated_at) \
             VALUES (?, ?, 'openai', 'gpt-4o', 1000, 0, 0, ?)",
        )
        .bind(&id)
        .bind(auth_level)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        id
    }

    async fn seed_mail(state: &AppState, id: &str, account_id: &str) {
        let now = now_unix();
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, subject, from_name, from_email, \
                 to_addrs, date_sent, date_received, body_text, created_at, updated_at) \
             VALUES (?, ?, ?, 'Renewal terms', 'Daniel Reyes', 'daniel@vendorco.example', \
                 '[{\"name\":\"\",\"email\":\"me@example.com\"}]', ?, ?, \
                 'Could you confirm the renewal terms we discussed?', 0, 0)",
        )
        .bind(id)
        .bind(account_id)
        .bind(format!("<{id}@x>"))
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    fn register_mock(state: &AppState) -> Arc<MockProvider> {
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        state.ai.register(mock.clone());
        mock
    }

    fn good_reply() -> Result<ChatResponse, ProviderError> {
        Ok(ChatResponse {
            text: "Hi Daniel,\n\nThanks for the update — the plan works for us and we will \
                   follow up with next steps this week.\n\nBest,\nMaya"
                .into(),
            finish: FinishReason::Stop,
            usage: TokenUsage {
                prompt_tokens: 120,
                completion_tokens: 60,
            },
            model_echo: "gpt-4o".into(),
            latency_ms: 700,
        })
    }

    fn money_reply() -> Result<ChatResponse, ProviderError> {
        Ok(ChatResponse {
            text: "Hi Daniel,\n\nWe can close this out for $50,000 by Friday.\n\nBest,\nMaya"
                .into(),
            finish: FinishReason::Stop,
            usage: TokenUsage::default(),
            model_echo: "gpt-4o".into(),
            latency_ms: 700,
        })
    }

    /// Default mock answers: E4 LLM ("normal"), needs-reply ("yes"), then the
    /// generation response. The canned default works for the first two; the
    /// generation response is scripted explicitly where the body matters.
    #[tokio::test]
    async fn passing_draft_is_queued_with_send_after() {
        let (state, _rx) = AppState::test_state().await;
        let mock = register_mock(&state);
        let account = seed_account(&state, 3).await;
        seed_mail(&state, "m1", &account).await;
        // E4 + needs-reply consume the canned default; the generation call
        // must yield a clean reply body, so script call #3.
        mock.push_chat(Ok(ChatResponse {
            text: "normal".into(),
            finish: FinishReason::Stop,
            usage: TokenUsage::default(),
            model_echo: "gpt-4o".into(),
            latency_ms: 10,
        }));
        mock.push_chat(Ok(ChatResponse {
            text: "yes".into(),
            finish: FinishReason::Stop,
            usage: TokenUsage::default(),
            model_echo: "gpt-4o".into(),
            latency_ms: 10,
        }));
        mock.push_chat(good_reply());

        let outcome = run_e3_for_mail(&state, "m1", &account).await.unwrap();
        let E3Outcome::Queued { draft_id } = outcome else {
            panic!("expected Queued, got {outcome:?}");
        };
        let (status, send_after, mode): (String, Option<i64>, String) =
            sqlx::query_as("SELECT status, send_after, trigger_mode FROM ai_drafts WHERE id = ?")
                .bind(&draft_id)
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        assert_eq!(status, "pending");
        assert_eq!(mode, "E3_auto");
        assert!(send_after.unwrap() > now_unix());
    }

    #[tokio::test]
    async fn self_check_violation_demotes_to_review() {
        let (state, _rx) = AppState::test_state().await;
        let mock = register_mock(&state);
        let account = seed_account(&state, 3).await;
        seed_mail(&state, "m1", &account).await;
        mock.push_chat(Ok(ChatResponse {
            text: "normal".into(),
            finish: FinishReason::Stop,
            usage: TokenUsage::default(),
            model_echo: "gpt-4o".into(),
            latency_ms: 10,
        }));
        mock.push_chat(Ok(ChatResponse {
            text: "yes".into(),
            finish: FinishReason::Stop,
            usage: TokenUsage::default(),
            model_echo: "gpt-4o".into(),
            latency_ms: 10,
        }));
        // Unprompted $50,000 → ContentViolation → demote.
        mock.push_chat(money_reply());

        let outcome = run_e3_for_mail(&state, "m1", &account).await.unwrap();
        let E3Outcome::Demoted { draft_id } = outcome else {
            panic!("expected Demoted, got {outcome:?}");
        };
        let draft_id = draft_id.unwrap();
        let (status, send_after): (String, Option<i64>) =
            sqlx::query_as("SELECT status, send_after FROM ai_drafts WHERE id = ?")
                .bind(&draft_id)
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        assert_eq!(status, "pending");
        assert_eq!(send_after, None);
        let (downgrades,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM ai_decisions WHERE decision_type = 'downgrade_e3_to_e2'",
        )
        .fetch_one(state.storage.db().pool())
        .await
        .unwrap();
        assert_eq!(downgrades, 1);
    }

    #[tokio::test]
    async fn gate_not_met_demotes_to_e2_review_draft() {
        let (state, _rx) = AppState::test_state().await;
        register_mock(&state);
        let account = seed_account(&state, 3).await;
        // Raise the gate so zero approved drafts fails it.
        sqlx::query("UPDATE account_ai_settings SET e3_min_history = 3 WHERE account_id = ?")
            .bind(&account)
            .execute(state.storage.db().pool())
            .await
            .unwrap();
        seed_mail(&state, "m1", &account).await;

        let outcome = run_e3_for_mail(&state, "m1", &account).await.unwrap();
        let E3Outcome::Demoted { draft_id } = outcome else {
            panic!("expected Demoted, got {outcome:?}");
        };
        let (mode,): (String,) = sqlx::query_as("SELECT trigger_mode FROM ai_drafts WHERE id = ?")
            .bind(draft_id.unwrap())
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        assert_eq!(mode, "E2_semi", "demoted drafts are review drafts");
        let (sent,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM mails WHERE folder = 'SENT'")
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        assert_eq!(sent, 0);
    }

    #[tokio::test]
    async fn rate_limited_account_demotes() {
        let (state, _rx) = AppState::test_state().await;
        register_mock(&state);
        let account = seed_account(&state, 3).await;
        seed_mail(&state, "m1", &account).await;
        let now = now_unix();
        for _ in 0..50 {
            sqlx::query(
                "INSERT INTO ai_decisions (id, account_id, decision_type, impact, \
                     action_description, result_description, created_at) \
                 VALUES (?, ?, 'auto_reply_sent', 'reply', 'Auto sent.', 'Sent.', ?)",
            )
            .bind(new_uuid())
            .bind(&account)
            .bind(now - 100)
            .execute(state.storage.db().pool())
            .await
            .unwrap();
        }

        let outcome = run_e3_for_mail(&state, "m1", &account).await.unwrap();
        assert!(matches!(outcome, E3Outcome::Demoted { .. }));
        let (queued,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM ai_drafts WHERE send_after IS NOT NULL")
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        assert_eq!(queued, 0, "rate-limited mail must not enter the send queue");
    }

    #[tokio::test]
    async fn loop_detection_discards_and_emits() {
        let (state, _rx) = AppState::test_state().await;
        register_mock(&state);
        let account = seed_account(&state, 3).await;
        let now = now_unix();
        sqlx::query(
            "INSERT INTO threads (id, account_id, subject, participants, latest_date, \
                 created_at, updated_at) VALUES ('t1', ?, 'Re: ping', '[]', ?, ?, ?)",
        )
        .bind(&account)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        // Four of our own sent mails already in the thread → loop.
        for i in 0..4 {
            sqlx::query(
                "INSERT INTO mails (id, account_id, thread_id, message_id, subject, from_email, \
                     to_addrs, date_sent, date_received, is_sent, created_at, updated_at) \
                 VALUES (?, ?, 't1', ?, 'Re: ping', 'me@example.com', '[]', ?, ?, 1, 0, 0)",
            )
            .bind(format!("sent{i}"))
            .bind(&account)
            .bind(format!("<sent{i}@x>"))
            .bind(now)
            .bind(now)
            .execute(state.storage.db().pool())
            .await
            .unwrap();
        }
        // The incoming fifth mail.
        sqlx::query(
            "INSERT INTO mails (id, account_id, thread_id, message_id, subject, from_email, \
                 to_addrs, date_sent, date_received, body_text, created_at, updated_at) \
             VALUES ('m1', ?, 't1', '<m1@x>', 'Re: ping', 'daniel@vendorco.example', \
                 '[{\"name\":\"\",\"email\":\"me@example.com\"}]', ?, ?, 'ping again', 0, 0)",
        )
        .bind(&account)
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();

        let outcome = run_e3_for_mail(&state, "m1", &account).await.unwrap();
        assert_eq!(outcome, E3Outcome::Discarded);
        let (drafts,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ai_drafts")
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        assert_eq!(drafts, 0, "no draft is generated for a looping thread");
    }

    #[tokio::test]
    async fn kill_switch_runs_the_mail_as_e2() {
        let (state, _rx) = AppState::test_state().await;
        register_mock(&state);
        let account = seed_account(&state, 3).await;
        seed_mail(&state, "m1", &account).await;
        SettingRepo::new(state.storage.db())
            .set(E3_PAUSED_UNTIL_KEY, &(now_unix() + 86_400).to_string())
            .await
            .unwrap();

        let outcome = run_e3_for_mail(&state, "m1", &account).await.unwrap();
        let E3Outcome::Demoted { draft_id } = outcome else {
            panic!("expected Demoted, got {outcome:?}");
        };
        let (mode, send_after): (String, Option<i64>) =
            sqlx::query_as("SELECT trigger_mode, send_after FROM ai_drafts WHERE id = ?")
                .bind(draft_id.unwrap())
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        assert_eq!(mode, "E2_semi");
        assert_eq!(send_after, None);
    }

    #[tokio::test]
    async fn semi_account_is_skipped_by_e3() {
        let (state, _rx) = AppState::test_state().await;
        let mock = register_mock(&state);
        let account = seed_account(&state, 2).await;
        seed_mail(&state, "m1", &account).await;
        let outcome = run_e3_for_mail(&state, "m1", &account).await.unwrap();
        assert_eq!(outcome, E3Outcome::Skipped);
        assert_eq!(mock.chat_call_count(), 0);
    }

    #[tokio::test]
    async fn kill_switch_value_parsing() {
        let (state, _rx) = AppState::test_state().await;
        let repo = SettingRepo::new(state.storage.db());
        assert!(!e3_paused(&state, 1_000).await.unwrap());
        repo.set(E3_PAUSED_UNTIL_KEY, "0").await.unwrap();
        assert!(!e3_paused(&state, 1_000).await.unwrap());
        repo.set(E3_PAUSED_UNTIL_KEY, "2000").await.unwrap();
        assert!(e3_paused(&state, 1_000).await.unwrap());
        assert!(!e3_paused(&state, 3_000).await.unwrap());
        // Quoted JSON-string form is tolerated.
        repo.set(E3_PAUSED_UNTIL_KEY, "\"2000\"").await.unwrap();
        assert!(e3_paused(&state, 1_000).await.unwrap());
        repo.set(E3_PAUSED_UNTIL_KEY, "not-a-number").await.unwrap();
        assert!(!e3_paused(&state, 1_000).await.unwrap());
    }
}
