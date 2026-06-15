//! Draft generation engine (T077 E1; shared by T082 E2, T084 E4, T085 E3).
//!
//! [`generate_and_store`] is the ONE generation path: prompt assembly via
//! T079, one provider call (with the dev/06 §6 single draft retry), body
//! cleanup (T077 cleaner), then one transaction inserting the `ai_drafts` row
//! and its append-only `ai_decisions` audit record, followed by the
//! `draft:ready` event. [`generate_e1`] wraps it for the explicit user
//! trigger; the E2/E3 pipelines and the E4 forced-draft route call it
//! directly with their own trigger mode. E1 runs at every authorization
//! level — only a fully disabled provider (`ai_provider = 'none'`) blocks it
//! (T087); the automatic pipelines gate on `resolve_auth_route` themselves.
//!
//! [`regenerate`] reuses the same path for "try again" (F_E1 §4.6): the fresh
//! draft is generated first, then the old one is marked
//! `discarded`/`superseded` — a provider failure therefore leaves the
//! previous draft untouched.
//!
//! Log safety (09 §5): identifiers, counts, token figures, and latencies only
//! — never draft bodies, subjects, or addresses.

use serde_json::json;

use crate::ai::audit::{decision_type, AuditEntry};
use crate::ai::provider::chat_with_retry;
use crate::ai::settings::{resolve_auth_route, AuthRouteDecision};
use crate::ai::types::Capability;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage::map_sqlx_err;
use crate::types::AiDraft;
use crate::util::{new_uuid, now_unix};

use super::cleaner::clean_ai_body;
use super::prompt_builder::{DraftPromptBuilder, TriggerMode};
// Re-exported so existing call sites (and tests) keep one canonical path; the
// values themselves moved to the repo (T080: single source for expiry config).
pub use super::repo::{DEFAULT_DRAFT_EXPIRY_HOURS, DRAFT_EXPIRY_HOURS_KEY};

/// The trigger-mail columns the engine needs.
#[derive(sqlx::FromRow)]
struct TriggerMailRow {
    account_id: String,
    subject: String,
    from_name: Option<String>,
    from_email: String,
    is_sent: i64,
}

/// Mode-specific `ai_decisions.action_description` for the shared generation
/// path. English summaries only — never mail or draft content (09 §5).
fn action_description_for(mode: TriggerMode) -> &'static str {
    match mode {
        TriggerMode::E1Manual => {
            "Generated a reply draft on explicit user request (E1 manual mode)."
        }
        TriggerMode::E2Semi => {
            "Generated a reply draft automatically after the needs-reply check (E2 semi-auto mode)."
        }
        TriggerMode::E3Auto => {
            "Generated a reply draft for the full-auto send pipeline (E3 auto mode)."
        }
    }
}

/// Generate an E1 manual reply draft for one received mail (F_E1 §4).
///
/// Errors: `NOT_FOUND` (mail missing, deleted, or a sent mail),
/// `AI_PROVIDER_UNREACHABLE` (no provider configured, or the provider is
/// down), `AI_CONTEXT_TOO_LONG`, `AI_RATE_LIMITED`. On any error nothing is
/// written.
pub async fn generate_e1(
    state: &AppState,
    mail_id: &str,
    instruction: Option<&str>,
) -> AppResult<AiDraft> {
    let pool = state.storage.db().pool();

    // E1 runs at every auth level (user-triggered, T087); only a fully
    // disabled provider blocks it — surfaced as AI_PROVIDER_UNREACHABLE so the
    // UI routes the user to provider setup (F_E1 §4.4). The account id comes
    // from a slim lookup; the shared path re-reads the full trigger row.
    let account: Option<(String,)> =
        sqlx::query_as("SELECT account_id FROM mails WHERE id = ? AND is_deleted = 0")
            .bind(mail_id)
            .fetch_optional(pool)
            .await
            .map_err(map_sqlx_err)?;
    let (account_id,) = account.ok_or(AppError::NotFound)?;
    if resolve_auth_route(state.storage.db(), &account_id).await? == AuthRouteDecision::Disabled {
        return Err(AppError::AiUnreachable(
            "no ai provider is configured for this account".into(),
        ));
    }

    generate_and_store(state, mail_id, TriggerMode::E1Manual, instruction).await
}

/// The ONE generation path shared by E1 (T077), E2 (T082), the E4 forced
/// draft (T084), and E3 (T085): prompt assembly (T079) → provider call with
/// the draft retry policy → body cleanup → `ai_drafts` insert +
/// `draft_created` audit record in one transaction → `draft:ready` event.
///
/// Authorization gating is the *caller's* responsibility — E1 checks
/// `Disabled`, the E2/E3 pipelines branch on `resolve_auth_route` before
/// calling in. Errors: `NOT_FOUND` (mail missing, deleted, or a sent mail),
/// `AI_PROVIDER_UNREACHABLE`, `AI_CONTEXT_TOO_LONG`, `AI_RATE_LIMITED`. On
/// any error nothing is written.
pub(crate) async fn generate_and_store(
    state: &AppState,
    mail_id: &str,
    trigger_mode: TriggerMode,
    instruction: Option<&str>,
) -> AppResult<AiDraft> {
    let pool = state.storage.db().pool();

    // 1) Trigger mail — replies are generated for received mails only.
    let mail: Option<TriggerMailRow> = sqlx::query_as(
        "SELECT account_id, subject, from_name, from_email, is_sent \
         FROM mails WHERE id = ? AND is_deleted = 0",
    )
    .bind(mail_id)
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx_err)?;
    let mail = mail.ok_or(AppError::NotFound)?;
    if mail.is_sent != 0 {
        return Err(AppError::NotFound);
    }

    // 2) Prompt assembly (T079) + provider call with the draft retry policy.
    let built =
        DraftPromptBuilder::build(state, mail_id, &mail.account_id, trigger_mode, instruction)
            .await?;
    let client = state
        .ai
        .resolve(&mail.account_id, Capability::DraftReply)
        .await?;
    let response = chat_with_retry(client.as_ref(), built.request.clone())
        .await
        .map_err(AppError::from)?;

    // 3) Cleanup: fences out, duplicated trailing signature out (the
    // account's role description is the signature hint, T077 §3).
    let signature_hint: Option<(Option<String>,)> =
        sqlx::query_as("SELECT role_description FROM accounts WHERE id = ?")
            .bind(&mail.account_id)
            .fetch_optional(pool)
            .await
            .map_err(map_sqlx_err)?;
    let signature_hint = signature_hint.and_then(|(d,)| d);
    let cleaned = clean_ai_body(&response.text, signature_hint.as_deref());

    // 4–5) Draft row + audit record in one transaction — both routed through
    // the single INSERT statements in draft::repo and audit::repo (T080/T088).
    let draft_id = new_uuid();
    let now = now_unix();
    let expiry_hours = super::repo::draft_expiry_hours(state.storage.db()).await?;
    let expires_at = (expiry_hours > 0).then(|| now + expiry_hours * 3_600);
    let to_addr_json = json!({
        "name": mail.from_name.clone().unwrap_or_default(),
        "email": mail.from_email,
    })
    .to_string();
    let knowledge_refs_json =
        serde_json::to_string(&built.knowledge_refs).unwrap_or_else(|_| "[]".into());
    let ai_model = if response.model_echo.is_empty() {
        built.request.model.clone()
    } else {
        response.model_echo.clone()
    };

    let mut tx = pool.begin().await.map_err(map_sqlx_err)?;
    super::repo::insert_draft_tx(
        &mut tx,
        &super::repo::NewAiDraft {
            id: &draft_id,
            trigger_mail_id: mail_id,
            account_id: &mail.account_id,
            to_addr_json: &to_addr_json,
            subject: &reply_subject(&mail.subject),
            body: &cleaned,
            trigger_mode: trigger_mode.as_str(),
            ai_model: &ai_model,
            knowledge_refs_json: &knowledge_refs_json,
            expires_at,
            created_at: now,
        },
    )
    .await?;
    crate::ai::audit::repo::insert_decision_tx(
        &mut tx,
        &AuditEntry {
            account_id: mail.account_id.clone(),
            mail_id: Some(mail_id.to_string()),
            draft_id: Some(draft_id.clone()),
            decision_type: decision_type::DRAFT_CREATED.to_string(),
            impact: "reply".into(),
            action_description: action_description_for(trigger_mode).into(),
            result_description: "Draft stored with status pending, awaiting human review.".into(),
            knowledge_refs: built.knowledge_refs.clone(),
            knowledge_summary: None,
            ai_model: Some(ai_model.clone()),
            input_tokens: Some(i64::from(response.usage.prompt_tokens)),
            output_tokens: Some(i64::from(response.usage.completion_tokens)),
            latency_ms: Some(i64::from(response.latency_ms)),
        },
    )
    .await?;
    tx.commit().await.map_err(map_sqlx_err)?;

    // 6) Notify the UI (T078 opens the compose window from this; the E6
    // review queue picks E2/E3 drafts up from the same event).
    state
        .events
        .draft_ready(&draft_id, mail_id, trigger_mode.as_str(), &mail.account_id);
    tracing::info!(
        event = "ai_draft_generated",
        draft_id = %draft_id,
        mail_id = %mail_id,
        account_id = %mail.account_id,
        trigger_mode = trigger_mode.as_str(),
        latency_ms = response.latency_ms,
        input_tokens = response.usage.prompt_tokens,
        output_tokens = response.usage.completion_tokens,
        knowledge_ref_count = built.knowledge_refs.len(),
        style_was_fallback = built.style_was_fallback,
        "ai reply draft generated"
    );

    // 7) Return the persisted row.
    load_draft(state, &draft_id).await
}

/// Regenerate a draft (F_E1 §4.6): produce a fresh draft for the same trigger
/// mail, then mark the old one `discarded`/`superseded`. Generation runs
/// first so a provider failure leaves the existing draft intact. Errors:
/// `NOT_FOUND` plus everything [`generate_e1`] can raise.
pub async fn regenerate(
    state: &AppState,
    draft_id: &str,
    instruction: Option<&str>,
) -> AppResult<AiDraft> {
    let pool = state.storage.db().pool();
    let row: Option<(String,)> =
        sqlx::query_as("SELECT trigger_mail_id FROM ai_drafts WHERE id = ?")
            .bind(draft_id)
            .fetch_optional(pool)
            .await
            .map_err(map_sqlx_err)?;
    let (trigger_mail_id,) = row.ok_or(AppError::NotFound)?;

    let fresh = generate_e1(state, &trigger_mail_id, instruction).await?;

    super::repo::mark_discarded(state.storage.db(), draft_id, "superseded").await?;

    tracing::info!(
        event = "draft_superseded",
        old_draft_id = %draft_id,
        new_draft_id = %fresh.id,
        "draft regenerated; previous draft discarded as superseded"
    );
    Ok(fresh)
}

/// Read one `ai_drafts` row as the wire DTO (thin wrapper over
/// [`super::repo::get`]). `NOT_FOUND` when absent.
pub async fn load_draft(state: &AppState, draft_id: &str) -> AppResult<AiDraft> {
    super::repo::get(state.storage.db(), draft_id).await
}

/// Reply subject: prefix `Re: ` unless the original already carries one.
fn reply_subject(original: &str) -> String {
    let trimmed = original.trim();
    if trimmed.to_lowercase().starts_with("re:") {
        trimmed.to_string()
    } else {
        format!("Re: {trimmed}")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ai::mock::MockProvider;
    use crate::ai::provider::ProviderError;
    use crate::ai::types::{ChatResponse, FinishReason, TokenUsage};
    use crate::types::{AiProvider, ErrorCode};
    use crate::util::truncate_chars;

    async fn seed_account(state: &AppState, ai_provider: &str) -> String {
        let id = new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
                 role_type, role_description, created_at, updated_at) \
             VALUES (?, ?, 'Maya Chen', 'imap', 'slate', 'W', 'work', \
                 'Coordinate vendor contracts and renewals.', ?, ?)",
        )
        .bind(&id)
        .bind(format!("{id}@example.com"))
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO account_ai_settings (account_id, auth_level, ai_provider, ai_model, updated_at) \
             VALUES (?, 1, ?, 'gpt-4o', ?)",
        )
        .bind(&id)
        .bind(ai_provider)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        id
    }

    async fn seed_mail(state: &AppState, id: &str, account_id: &str, is_sent: i64) {
        let body = "Could you confirm the renewal terms we discussed last week?";
        let now = now_unix();
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, subject, from_name, from_email, \
                 to_addrs, date_sent, date_received, body_text, snippet, is_sent, \
                 created_at, updated_at) \
             VALUES (?, ?, ?, 'Renewal terms', 'Daniel Reyes', 'daniel@vendorco.example', \
                 '[]', ?, ?, ?, ?, ?, 0, 0)",
        )
        .bind(id)
        .bind(account_id)
        .bind(format!("<{id}@x>"))
        .bind(now)
        .bind(now)
        .bind(body)
        .bind(truncate_chars(body, 200))
        .bind(is_sent)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    fn register_mock(state: &AppState) -> Arc<MockProvider> {
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        state.ai.register(mock.clone());
        mock
    }

    fn ok_response(text: &str) -> Result<ChatResponse, ProviderError> {
        Ok(ChatResponse {
            text: text.into(),
            finish: FinishReason::Stop,
            usage: TokenUsage {
                prompt_tokens: 120,
                completion_tokens: 60,
            },
            model_echo: "gpt-4o-2024-08-06".into(),
            latency_ms: 850,
        })
    }

    async fn draft_count(state: &AppState) -> i64 {
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ai_drafts")
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        count
    }

    #[tokio::test]
    async fn happy_path_writes_draft_decision_and_returns_dto() {
        let (state, _rx) = AppState::test_state().await;
        let mock = register_mock(&state);
        mock.push_chat(ok_response(
            "Hi Daniel,\n\nHappy to confirm the renewal terms as discussed.",
        ));
        let account = seed_account(&state, "openai").await;
        seed_mail(&state, "m1", &account, 0).await;

        let draft = generate_e1(&state, "m1", None).await.unwrap();

        assert_eq!(draft.trigger_mail_id, "m1");
        assert_eq!(draft.account_id, account);
        assert_eq!(draft.status, "pending");
        assert_eq!(draft.trigger_mode, "E1_manual");
        assert_eq!(draft.subject, "Re: Renewal terms");
        assert_eq!(draft.to_addr.email, "daniel@vendorco.example");
        assert_eq!(draft.to_addr.name.as_deref(), Some("Daniel Reyes"));
        assert!(draft.body_current.contains("Happy to confirm"));
        assert_eq!(draft.body_original, draft.body_current);
        assert_eq!(draft.ai_model, "gpt-4o-2024-08-06");
        assert!(!draft.is_edited);
        // Default 72-hour expiry.
        assert_eq!(draft.expires_at, Some(draft.created_at + 72 * 3_600));

        // Audit row: draft_created, token/latency figures, no body content.
        let (decision_type, impact, action, result, latency): (
            String,
            String,
            String,
            String,
            i64,
        ) = sqlx::query_as(
            "SELECT decision_type, impact, action_description, result_description, latency_ms \
                 FROM ai_decisions WHERE draft_id = ?",
        )
        .bind(&draft.id)
        .fetch_one(state.storage.db().pool())
        .await
        .unwrap();
        assert_eq!(decision_type, "draft_created");
        assert_eq!(impact, "reply");
        assert_eq!(latency, 850);
        assert!(!action.contains("Happy to confirm"));
        assert!(!result.contains("Happy to confirm"));
    }

    #[tokio::test]
    async fn fences_are_stripped_from_the_stored_body() {
        let (state, _rx) = AppState::test_state().await;
        let mock = register_mock(&state);
        mock.push_chat(ok_response("```\nHi Daniel,\n\nConfirmed for Monday.\n```"));
        let account = seed_account(&state, "openai").await;
        seed_mail(&state, "m1", &account, 0).await;

        let draft = generate_e1(&state, "m1", None).await.unwrap();
        assert!(!draft.body_current.contains("```"));
        assert!(draft.body_current.starts_with("Hi Daniel,"));
    }

    #[tokio::test]
    async fn provider_down_is_unreachable_and_writes_nothing() {
        let (state, _rx) = AppState::test_state().await;
        let mock = register_mock(&state);
        mock.set_default_chat_error(ProviderError::Unreachable("connect refused".into()));
        let account = seed_account(&state, "openai").await;
        seed_mail(&state, "m1", &account, 0).await;

        let err = generate_e1(&state, "m1", None).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::AiProviderUnreachable);
        assert_eq!(draft_count(&state).await, 0);
        let (decisions,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ai_decisions")
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        assert_eq!(decisions, 0);
    }

    #[tokio::test]
    async fn provider_none_is_unreachable_before_any_call() {
        let (state, _rx) = AppState::test_state().await;
        let mock = register_mock(&state);
        let account = seed_account(&state, "none").await;
        seed_mail(&state, "m1", &account, 0).await;

        let err = generate_e1(&state, "m1", None).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::AiProviderUnreachable);
        assert_eq!(mock.chat_call_count(), 0);
        assert_eq!(draft_count(&state).await, 0);
    }

    #[tokio::test]
    async fn missing_or_sent_mail_is_not_found() {
        let (state, _rx) = AppState::test_state().await;
        register_mock(&state);
        let account = seed_account(&state, "openai").await;
        seed_mail(&state, "sent1", &account, 1).await;

        let err = generate_e1(&state, "missing", None).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::NotFound);
        let err = generate_e1(&state, "sent1", None).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert_eq!(draft_count(&state).await, 0);
    }

    #[tokio::test]
    async fn zero_expiry_setting_disables_expiry() {
        let (state, _rx) = AppState::test_state().await;
        register_mock(&state);
        let account = seed_account(&state, "openai").await;
        seed_mail(&state, "m1", &account, 0).await;
        crate::storage::SettingRepo::new(state.storage.db())
            .set(DRAFT_EXPIRY_HOURS_KEY, "0")
            .await
            .unwrap();

        let draft = generate_e1(&state, "m1", None).await.unwrap();
        assert_eq!(draft.expires_at, None);
    }

    #[tokio::test]
    async fn regenerate_supersedes_the_old_draft() {
        let (state, _rx) = AppState::test_state().await;
        register_mock(&state);
        let account = seed_account(&state, "openai").await;
        seed_mail(&state, "m1", &account, 0).await;

        let first = generate_e1(&state, "m1", None).await.unwrap();
        let second = regenerate(&state, &first.id, Some("Shorter, please."))
            .await
            .unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(second.status, "pending");

        let old = load_draft(&state, &first.id).await.unwrap();
        assert_eq!(old.status, "discarded");
        assert_eq!(old.discard_reason.as_deref(), Some("superseded"));
        assert!(old.discarded_at.is_some());
    }

    #[tokio::test]
    async fn chained_regenerations_leave_one_pending_draft() {
        let (state, _rx) = AppState::test_state().await;
        register_mock(&state);
        let account = seed_account(&state, "openai").await;
        seed_mail(&state, "m1", &account, 0).await;

        let mut current = generate_e1(&state, "m1", None).await.unwrap();
        for _ in 0..3 {
            current = regenerate(&state, &current.id, None).await.unwrap();
        }
        let (pending,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM ai_drafts WHERE status = 'pending'")
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        assert_eq!(pending, 1);
        assert_eq!(
            load_draft(&state, &current.id).await.unwrap().status,
            "pending"
        );
    }

    #[tokio::test]
    async fn regenerate_failure_keeps_the_old_draft_pending() {
        let (state, _rx) = AppState::test_state().await;
        let mock = register_mock(&state);
        let account = seed_account(&state, "openai").await;
        seed_mail(&state, "m1", &account, 0).await;

        let first = generate_e1(&state, "m1", None).await.unwrap();
        mock.set_default_chat_error(ProviderError::Unreachable("link down".into()));
        let err = regenerate(&state, &first.id, None).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::AiProviderUnreachable);
        assert_eq!(
            load_draft(&state, &first.id).await.unwrap().status,
            "pending",
            "a failed regeneration must not discard the existing draft"
        );
    }

    #[tokio::test]
    async fn regenerate_unknown_draft_is_not_found() {
        let (state, _rx) = AppState::test_state().await;
        let err = regenerate(&state, "missing", None).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::NotFound);
    }

    #[test]
    fn reply_subject_prefixes_once() {
        assert_eq!(reply_subject("Renewal terms"), "Re: Renewal terms");
        assert_eq!(reply_subject("Re: Renewal terms"), "Re: Renewal terms");
        assert_eq!(reply_subject("RE: Renewal terms"), "RE: Renewal terms");
    }
}
