//! E2 semi-auto generation pipeline (T082 §3, F_E2 §4, AI_MODES §4.2).
//!
//! `run_e2_for_mail` is the per-mail entry the background worker calls after
//! ingest. Sequence: auth-route gate (`Semi` only) → idempotency check →
//! concurrency permits (global 4 / per-account 2) → E4 pre-scan checkpoint
//! (T084) → needs-reply decision (rule chain + LLM) → shared generation path
//! (insert + audit + `draft:ready`).
//!
//! Skips are normal states, not errors: a Manual account, a duplicate
//! trigger, a trashed/sensitive E4 outcome, and a "no reply needed" verdict
//! all return `Ok(None)` quietly (debug logs, identifiers only).

use crate::ai::draft::prompt_builder::TriggerMode;
use crate::ai::settings::{resolve_auth_route, AuthRouteDecision};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage::map_sqlx_err;
use crate::types::AiDraft;

use super::e4_classifier::{self, E4Outcome};
use super::e4_router::{self, MailRouteDecision};
use super::{account_email, load_mail, needs_reply};

/// Whether a live (non-discarded, non-expired) draft already exists for the
/// trigger mail (T082 §6 idempotency query).
pub async fn has_live_draft(state: &AppState, mail_id: &str) -> AppResult<bool> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM ai_drafts \
         WHERE trigger_mail_id = ? AND status NOT IN ('discarded', 'expired')",
    )
    .bind(mail_id)
    .fetch_one(state.storage.db().pool())
    .await
    .map_err(map_sqlx_err)?;
    Ok(count > 0)
}

/// E4 checkpoint shared by the E2 and E3 pipelines (T084 §3): classify, then
/// route. `Proceed` lets the caller continue; anything else means the mail
/// was consumed (trashed or force-drafted).
pub(super) async fn e4_checkpoint(
    state: &AppState,
    mail: &super::PipelineMail,
) -> AppResult<MailRouteDecision> {
    let ctx = e4_classifier::load_context(state, mail).await?;
    // Provider resolution failure = "not configured" → rules-only (F_E4 §6).
    let client = state
        .ai
        .resolve(&mail.account_id, crate::ai::types::Capability::RiskReason)
        .await
        .ok();
    let outcome = e4_classifier::classify(mail, &ctx, client.as_deref()).await;
    if outcome == E4Outcome::Normal {
        return Ok(MailRouteDecision::Proceed);
    }
    e4_router::route_mail(state, outcome, mail).await
}

/// Run the E2 semi-auto pipeline for one ingested mail. Returns the created
/// draft, or `None` for every skip path. Errors surface only for real
/// failures (DB errors, provider failure during generation) — the worker
/// `warn`-logs them and emits `pipeline:error`.
pub async fn run_e2_for_mail(
    state: &AppState,
    mail_id: &str,
    account_id: &str,
) -> AppResult<Option<AiDraft>> {
    // 1) Authorization gate (T087): E2 runs only for Semi accounts. Full-auto
    // accounts are served by the E3 pipeline; Manual/Disabled accounts skip.
    // A missing settings row is treated as "not configured" → skip.
    match resolve_auth_route(state.storage.db(), account_id).await {
        Ok(AuthRouteDecision::Semi) => {}
        Ok(_) => return Ok(None),
        Err(AppError::NotFound) => return Ok(None),
        Err(e) => return Err(e),
    }

    // 2) Idempotency: one live draft per trigger mail (T082 §6).
    if has_live_draft(state, mail_id).await? {
        tracing::debug!(
            event = "e2_skip_duplicate",
            mail_id = %mail_id,
            "e2 pipeline skipped; a live draft already exists"
        );
        return Ok(None);
    }

    // 3) Concurrency permits: global 4, per-account 2 (F_E2 §4.6). The global
    // permit is held by reference (released at fn exit); the per-account
    // permit is owned because the semaphore Arc comes out of the shared map.
    let _global = state
        .e2_semaphore
        .acquire()
        .await
        .map_err(|_| AppError::Internal(anyhow::anyhow!("e2 semaphore closed")))?;
    let account_sem = state.e2_account_sem(account_id);
    let _local = account_sem
        .acquire_owned()
        .await
        .map_err(|_| AppError::Internal(anyhow::anyhow!("e2 account semaphore closed")))?;

    // 4) Load the mail snapshot; sent or vanished mails skip.
    let Some(mail) = load_mail(state.storage.db(), mail_id).await? else {
        return Ok(None);
    };
    if mail.is_sent != 0 {
        return Ok(None);
    }

    // 5) E4 pre-scan checkpoint (T084): trash spam, force-draft sensitive.
    if e4_checkpoint(state, &mail).await? != MailRouteDecision::Proceed {
        return Ok(None);
    }

    // 6) Needs-reply decision (rule chain + cached LLM verdict).
    let own_email = account_email(state.storage.db(), account_id)
        .await?
        .unwrap_or_default();
    if !needs_reply::needs_reply(state, &mail, &own_email).await? {
        tracing::debug!(
            event = "e2_skip_no_reply",
            mail_id = %mail_id,
            account_id = %account_id,
            needs_reply = false,
            "e2 pipeline skipped; mail does not need a reply"
        );
        return Ok(None);
    }

    // 7) Generate through the shared path (insert + audit + draft:ready). On a
    // resumed mail, fold the operator's answer to the proactive query back in
    // (T096); a non-resumed mail has no answered query, so this is `None`.
    let instruction = super::resume::answer_instruction_for_mail(state, mail_id).await;
    let draft = crate::ai::draft::engine::generate_and_store(
        state,
        mail_id,
        TriggerMode::E2Semi,
        instruction.as_deref(),
    )
    .await?;
    Ok(Some(draft))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ai::mock::MockProvider;
    use crate::types::AiProvider;
    use crate::util::{new_uuid, now_unix};

    async fn seed_account(state: &AppState, auth_level: i64) -> String {
        let id = new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
                 role_type, role_description, auth_level, created_at, updated_at) \
             VALUES (?, 'me@example.com', 'Maya Chen', 'imap', 'slate', 'W', 'work', \
                 'Coordinate vendor contracts and renewals.', ?, ?, ?)",
        )
        .bind(&id)
        .bind(auth_level)
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO account_ai_settings (account_id, auth_level, ai_provider, ai_model, \
                 daily_query_limit, updated_at) VALUES (?, ?, 'openai', 'gpt-4o', 1000, ?)",
        )
        .bind(&id)
        .bind(auth_level)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        id
    }

    async fn seed_mail(state: &AppState, id: &str, account_id: &str, from_email: &str) {
        let now = now_unix();
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, subject, from_name, from_email, \
                 to_addrs, date_sent, date_received, body_text, created_at, updated_at) \
             VALUES (?, ?, ?, 'Renewal terms', 'Daniel Reyes', ?, \
                 '[{\"name\":\"\",\"email\":\"me@example.com\"}]', ?, ?, \
                 'Could you confirm the renewal terms we discussed?', 0, 0)",
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

    fn register_mock(state: &AppState) -> Arc<MockProvider> {
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        state.ai.register(mock.clone());
        mock
    }

    #[tokio::test]
    async fn happy_path_creates_an_e2_draft() {
        let (state, _rx) = AppState::test_state().await;
        let mock = register_mock(&state);
        let account = seed_account(&state, 2).await;
        seed_mail(&state, "m1", &account, "daniel@vendorco.example").await;
        // Classifier "yes", then E4 LLM "normal" or generation — the mock
        // answers every unscripted call with its canned success, which the
        // classifier parses as "yes" (conservative) and E4 as not-sensitive.
        let draft = run_e2_for_mail(&state, "m1", &account).await.unwrap();
        let draft = draft.expect("draft should be generated");
        assert_eq!(draft.trigger_mode, "E2_semi");
        assert_eq!(draft.status, "pending");
        assert_eq!(draft.trigger_mail_id, "m1");
        assert!(mock.chat_call_count() >= 2, "e4/needs-reply + generation");

        let (audits,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM ai_decisions WHERE decision_type = 'draft_created'",
        )
        .fetch_one(state.storage.db().pool())
        .await
        .unwrap();
        assert_eq!(audits, 1);
    }

    #[tokio::test]
    async fn duplicate_trigger_is_idempotent() {
        let (state, _rx) = AppState::test_state().await;
        register_mock(&state);
        let account = seed_account(&state, 2).await;
        seed_mail(&state, "m1", &account, "daniel@vendorco.example").await;

        let first = run_e2_for_mail(&state, "m1", &account).await.unwrap();
        assert!(first.is_some());
        let second = run_e2_for_mail(&state, "m1", &account).await.unwrap();
        assert!(second.is_none());

        let (pending,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM ai_drafts WHERE status = 'pending'")
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        assert_eq!(pending, 1);
    }

    #[tokio::test]
    async fn manual_account_is_skipped() {
        let (state, _rx) = AppState::test_state().await;
        let mock = register_mock(&state);
        let account = seed_account(&state, 1).await;
        seed_mail(&state, "m1", &account, "daniel@vendorco.example").await;

        let result = run_e2_for_mail(&state, "m1", &account).await.unwrap();
        assert!(result.is_none());
        assert_eq!(mock.chat_call_count(), 0);
    }

    #[tokio::test]
    async fn noreply_sender_generates_nothing() {
        let (state, _rx) = AppState::test_state().await;
        register_mock(&state);
        let account = seed_account(&state, 2).await;
        seed_mail(&state, "m1", &account, "noreply@shop.example").await;

        let result = run_e2_for_mail(&state, "m1", &account).await.unwrap();
        assert!(result.is_none());
        let (drafts,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ai_drafts")
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        assert_eq!(drafts, 0);
    }

    #[tokio::test]
    async fn sensitive_mail_short_circuits_into_the_forced_draft_path() {
        let (state, _rx) = AppState::test_state().await;
        register_mock(&state);
        let account = seed_account(&state, 2).await;
        seed_mail(&state, "m1", &account, "daniel@vendorco.example").await;
        // Mark the sender as an important contact → E4 hard rule.
        let now = now_unix();
        sqlx::query(
            "INSERT INTO contacts (id, email, first_seen_at, last_seen_at, is_trusted, \
                 created_at, updated_at) VALUES (?, 'daniel@vendorco.example', ?, ?, 1, ?, ?)",
        )
        .bind(new_uuid())
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();

        let result = run_e2_for_mail(&state, "m1", &account).await.unwrap();
        // The E2 pipeline itself returns None — the draft was created by the
        // E4 forced-draft route, with a risk event alongside it.
        assert!(result.is_none());
        let (risks,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM risk_events")
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        assert_eq!(risks, 1);
        let (mode,): (String,) =
            sqlx::query_as("SELECT trigger_mode FROM ai_drafts WHERE trigger_mail_id = 'm1'")
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        assert_eq!(mode, "E2_semi");
    }
}
