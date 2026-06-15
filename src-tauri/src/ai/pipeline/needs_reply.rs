//! "Needs reply" classifier for the E2/E3 pipelines (T082 §3, F_E2 §4.2).
//!
//! Two stages:
//!
//! 1. **Rule chain** (pure, no LLM): automated senders, newsletter / auto-reply
//!    subject markers, and CC-only delivery are all definitive "no reply".
//! 2. **LLM binary classifier** when no rule fires: temperature 0.0,
//!    `max_tokens = 10`, expecting `yes`/`no`. Any other answer — and any
//!    provider failure — defaults to **true** (conservative: better one extra
//!    draft than a missed reply). Verdicts are cached per thread for 24 h via
//!    `ai_decisions` rows (`decision_type = 'needs_reply_check'`,
//!    `result_description = 'yes' | 'no'`).
//!
//! Subject markers are ASCII-only plus the spec's CJK advertisement tag, which
//! is matched through `\u{..}` escapes — no raw CJK in source (repo language
//! rule).

use once_cell::sync::Lazy;
use regex::Regex;

use crate::ai::audit::{decision_type, AuditEntry};
use crate::ai::types::{Capability, ChatMessage, ChatRequest, ChatRole};
use crate::error::AppResult;
use crate::state::AppState;
use crate::storage::map_sqlx_err;
use crate::util::{now_unix, truncate_chars};

use super::PipelineMail;

/// Automated-sender local parts that never expect a human reply (F_E2 §4.2).
static AUTOMATED_SENDER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(no-?reply|notifications?|mailer-daemon|bounce|postmaster)\b")
        .expect("automated sender regex is valid")
});

/// Subject markers for bulk / automated mail. The bracketed CJK
/// "advertisement" tag from the spec is expressed via unicode escapes.
static NO_REPLY_SUBJECT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)newsletter|unsubscribe|promotion|auto-?reply|out of office|\[advertisement\]|\[\u{5E7F}\u{544A}\]",
    )
    .expect("subject marker regex is valid")
});

/// Classifier prompt budget — a one-word answer is all we want.
const CLASSIFIER_MAX_TOKENS: u32 = 10;
const CLASSIFIER_TEMPERATURE: f32 = 0.0;
/// Snippet length fed to the classifier (header + opening text only).
const CLASSIFIER_SNIPPET_CHARS: usize = 300;
/// Thread-scoped cache lifetime for an LLM verdict (F_E2 §4.2).
const CACHE_WINDOW_SECS: i64 = 86_400;

const CLASSIFIER_SYSTEM: &str = "You are an email triage classifier. Decide whether the \
incoming email expects a human reply. Answer with exactly one word: yes or no.";

/// Rule-chain verdict: `Some(false)` when a rule definitively says "no reply
/// needed", `None` when the rules are inconclusive (LLM decides). The rules
/// never produce `Some(true)` — only the classifier affirms.
pub fn rule_chain_verdict(mail: &PipelineMail, account_email: &str) -> Option<bool> {
    if AUTOMATED_SENDER_RE.is_match(&mail.from_email) {
        return Some(false);
    }
    if NO_REPLY_SUBJECT_RE.is_match(&mail.subject) {
        return Some(false);
    }
    // The account is only CC'd, not addressed directly → no reply expected.
    if !mail.to_contains(account_email) && mail.cc_contains(account_email) {
        return Some(false);
    }
    None
}

/// Read a cached thread-scoped verdict from `ai_decisions` (T082 §6).
async fn cached_verdict(state: &AppState, thread_id: &str, now: i64) -> AppResult<Option<bool>> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT result_description FROM ai_decisions \
         WHERE decision_type = 'needs_reply_check' \
           AND mail_id IN (SELECT id FROM mails WHERE thread_id = ?) \
           AND created_at > ? \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(thread_id)
    .bind(now - CACHE_WINDOW_SECS)
    .fetch_optional(state.storage.db().pool())
    .await
    .map_err(map_sqlx_err)?;
    Ok(row.map(|(verdict,)| verdict.trim().eq_ignore_ascii_case("yes")))
}

/// Map the model's one-word answer onto a verdict. Anything that is not a
/// clear "no" counts as "yes" (conservative policy, T082 §3).
fn parse_classifier_answer(text: &str) -> bool {
    !text.trim().to_lowercase().starts_with("no")
}

/// Full needs-reply decision for one mail: rule chain → 24 h thread cache →
/// LLM binary classifier → conservative default `true` on any provider
/// problem. The LLM verdict is persisted as a `needs_reply_check` audit row so
/// later mails in the same thread reuse it.
pub async fn needs_reply(
    state: &AppState,
    mail: &PipelineMail,
    account_email: &str,
) -> AppResult<bool> {
    if let Some(verdict) = rule_chain_verdict(mail, account_email) {
        tracing::debug!(
            event = "needs_reply_rule_hit",
            mail_id = %mail.id,
            needs_reply = verdict,
            "needs-reply decided by rule chain"
        );
        return Ok(verdict);
    }

    let now = now_unix();
    if let Some(thread_id) = mail.thread_id.as_deref() {
        if let Some(cached) = cached_verdict(state, thread_id, now).await? {
            tracing::debug!(
                event = "needs_reply_cache_hit",
                mail_id = %mail.id,
                needs_reply = cached,
                "needs-reply reused the thread-scoped verdict"
            );
            return Ok(cached);
        }
    }

    // LLM binary classification. Provider resolution or call failure → true
    // (conservative; F_E2 §4.2 — never silently drop a possible reply).
    let client = match state
        .ai
        .resolve(&mail.account_id, Capability::StyleProfile)
        .await
    {
        Ok(client) => client,
        Err(e) => {
            tracing::warn!(
                event = "needs_reply_provider_unavailable",
                mail_id = %mail.id,
                code = e.code().as_wire(),
                "needs-reply classifier has no provider; defaulting to true"
            );
            return Ok(true);
        }
    };
    let model = state
        .ai
        .account_config(&mail.account_id)
        .await
        .ok()
        .and_then(|cfg| cfg.model)
        .unwrap_or_default();
    let user_text = format!(
        "Subject: {}\n\n{}",
        mail.subject,
        truncate_chars(mail.text(), CLASSIFIER_SNIPPET_CHARS)
    );
    let request = ChatRequest {
        model,
        system: CLASSIFIER_SYSTEM.into(),
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: user_text,
        }],
        max_tokens: CLASSIFIER_MAX_TOKENS,
        temperature: CLASSIFIER_TEMPERATURE,
        stop: Vec::new(),
        purpose: Capability::StyleProfile,
        request_id: uuid::Uuid::new_v4(),
    };

    let verdict = match client.chat(request).await {
        Ok(response) => {
            let verdict = parse_classifier_answer(&response.text);
            // Persist the verdict so the same thread skips the LLM for 24 h.
            // `result_description` carries exactly "yes"/"no" — the cache key.
            state
                .audit
                .log_await(AuditEntry {
                    account_id: mail.account_id.clone(),
                    mail_id: Some(mail.id.clone()),
                    draft_id: None,
                    decision_type: decision_type::NEEDS_REPLY_CHECK.to_string(),
                    impact: "reply".into(),
                    action_description:
                        "Classified whether the incoming mail needs a human reply (E2 pipeline)."
                            .into(),
                    result_description: if verdict { "yes".into() } else { "no".into() },
                    knowledge_refs: Vec::new(),
                    knowledge_summary: None,
                    ai_model: Some(response.model_echo.clone()),
                    input_tokens: Some(i64::from(response.usage.prompt_tokens)),
                    output_tokens: Some(i64::from(response.usage.completion_tokens)),
                    latency_ms: Some(i64::from(response.latency_ms)),
                })
                .await?;
            verdict
        }
        Err(e) => {
            tracing::warn!(
                event = "needs_reply_llm_failed",
                mail_id = %mail.id,
                error_class = %e,
                "needs-reply classifier call failed; defaulting to true"
            );
            true
        }
    };
    Ok(verdict)
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

    fn mail(from_email: &str, subject: &str) -> PipelineMail {
        PipelineMail {
            id: "m1".into(),
            account_id: "acc".into(),
            thread_id: Some("t1".into()),
            subject: subject.into(),
            from_email: from_email.into(),
            to_addrs: r#"[{"name":"","email":"me@example.com"}]"#.into(),
            cc_addrs: "[]".into(),
            body_text: Some("Could you confirm the renewal terms?".into()),
            snippet: None,
            imap_flags: "[]".into(),
            spam_score: None,
            has_attachments: 0,
            is_sent: 0,
        }
    }

    fn answer(text: &str) -> Result<ChatResponse, ProviderError> {
        Ok(ChatResponse {
            text: text.into(),
            finish: FinishReason::Stop,
            usage: TokenUsage {
                prompt_tokens: 40,
                completion_tokens: 1,
            },
            model_echo: "gpt-4o".into(),
            latency_ms: 120,
        })
    }

    async fn seed_account(state: &AppState) -> String {
        let id = new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
                 created_at, updated_at) VALUES (?, 'me@example.com', 'Work', 'imap', 'slate', 'W', ?, ?)",
        )
        .bind(&id)
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO account_ai_settings (account_id, auth_level, ai_provider, ai_model, \
                 daily_query_limit, updated_at) VALUES (?, 2, 'openai', 'gpt-4o', 1000, ?)",
        )
        .bind(&id)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        id
    }

    async fn seed_thread_mail(state: &AppState, mail_id: &str, account_id: &str, thread: &str) {
        let now = now_unix();
        sqlx::query(
            "INSERT INTO threads (id, account_id, subject, participants, latest_date, \
                 created_at, updated_at) \
             VALUES (?, ?, 'Renewal terms', '[]', ?, ?, ?) ON CONFLICT(id) DO NOTHING",
        )
        .bind(thread)
        .bind(account_id)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO mails (id, account_id, thread_id, message_id, subject, from_email, \
                 to_addrs, date_sent, date_received, created_at, updated_at) \
             VALUES (?, ?, ?, ?, 'Renewal terms', 'boss@company.com', '[]', ?, ?, 0, 0)",
        )
        .bind(mail_id)
        .bind(account_id)
        .bind(thread)
        .bind(format!("<{mail_id}@x>"))
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    #[test]
    fn rule_chain_blocks_automated_senders_and_bulk_subjects() {
        let account = "me@example.com";
        assert_eq!(
            rule_chain_verdict(&mail("noreply@example.com", "Update"), account),
            Some(false)
        );
        assert_eq!(
            rule_chain_verdict(&mail("no-reply@example.com", "Update"), account),
            Some(false)
        );
        assert_eq!(
            rule_chain_verdict(&mail("mailer-daemon@example.com", "Failure"), account),
            Some(false)
        );
        assert_eq!(
            rule_chain_verdict(&mail("ana@x.com", "Weekly Newsletter"), account),
            Some(false)
        );
        assert_eq!(
            rule_chain_verdict(&mail("ana@x.com", "Out of Office: away"), account),
            Some(false)
        );
        assert_eq!(
            rule_chain_verdict(&mail("boss@company.com", "Renewal terms"), account),
            None
        );
    }

    #[test]
    fn rule_chain_blocks_cc_only_delivery() {
        let mut m = mail("boss@company.com", "FYI");
        m.to_addrs = r#"[{"name":"","email":"other@example.com"}]"#.into();
        m.cc_addrs = r#"[{"name":"","email":"me@example.com"}]"#.into();
        assert_eq!(rule_chain_verdict(&m, "me@example.com"), Some(false));
        // Not in TO nor CC (e.g. BCC delivery) → inconclusive, LLM decides.
        m.cc_addrs = "[]".into();
        assert_eq!(rule_chain_verdict(&m, "me@example.com"), None);
    }

    #[tokio::test]
    async fn rule_hit_never_calls_the_llm() {
        let (state, _rx) = AppState::test_state().await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        state.ai.register(mock.clone());
        let account = seed_account(&state).await;
        let mut m = mail("noreply@example.com", "Receipt");
        m.account_id = account;

        let verdict = needs_reply(&state, &m, "me@example.com").await.unwrap();
        assert!(!verdict);
        assert_eq!(mock.chat_call_count(), 0);
    }

    #[tokio::test]
    async fn llm_yes_and_no_are_respected() {
        let (state, _rx) = AppState::test_state().await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        state.ai.register(mock.clone());
        let account = seed_account(&state).await;

        seed_thread_mail(&state, "m1", &account, "t1").await;
        let mut m = mail("boss@company.com", "Renewal terms");
        m.account_id = account.clone();
        m.thread_id = Some("t1".into());
        m.id = "m1".into();

        mock.push_chat(answer("Yes"));
        assert!(needs_reply(&state, &m, "me@example.com").await.unwrap());

        // Different thread so the cached "yes" is not reused.
        seed_thread_mail(&state, "m2", &account, "t2").await;
        let mut m2 = m.clone();
        m2.id = "m2".into();
        m2.thread_id = Some("t2".into());
        mock.push_chat(answer("no"));
        assert!(!needs_reply(&state, &m2, "me@example.com").await.unwrap());

        // Garbage answer counts as yes (conservative).
        seed_thread_mail(&state, "m3", &account, "t3").await;
        let mut m3 = m.clone();
        m3.id = "m3".into();
        m3.thread_id = Some("t3".into());
        mock.push_chat(answer("cannot tell"));
        assert!(needs_reply(&state, &m3, "me@example.com").await.unwrap());
    }

    #[tokio::test]
    async fn provider_error_defaults_to_true() {
        let (state, _rx) = AppState::test_state().await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        state.ai.register(mock.clone());
        let account = seed_account(&state).await;
        seed_thread_mail(&state, "m1", &account, "t1").await;
        let mut m = mail("boss@company.com", "Renewal terms");
        m.account_id = account;

        mock.push_chat(Err(ProviderError::Unreachable("down".into())));
        assert!(needs_reply(&state, &m, "me@example.com").await.unwrap());
        // No cache row was written for the failed call.
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM ai_decisions WHERE decision_type = 'needs_reply_check'",
        )
        .fetch_one(state.storage.db().pool())
        .await
        .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn second_mail_in_thread_reuses_the_cached_verdict() {
        let (state, _rx) = AppState::test_state().await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        state.ai.register(mock.clone());
        let account = seed_account(&state).await;
        seed_thread_mail(&state, "m1", &account, "t1").await;
        seed_thread_mail(&state, "m2", &account, "t1").await;

        let mut first = mail("boss@company.com", "Renewal terms");
        first.account_id = account.clone();
        first.id = "m1".into();
        mock.push_chat(answer("no"));
        assert!(!needs_reply(&state, &first, "me@example.com").await.unwrap());
        assert_eq!(mock.chat_call_count(), 1);

        let mut second = first.clone();
        second.id = "m2".into();
        assert!(!needs_reply(&state, &second, "me@example.com")
            .await
            .unwrap());
        // Cache hit: still one LLM call.
        assert_eq!(mock.chat_call_count(), 1);
    }
}
