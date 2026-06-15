//! Draft prompt assembly shared by E1/E2/E3 (T079, F_E1 §4.2, F_E2 §4.3,
//! AI_MODES §4.1/§4.2/§10.1).
//!
//! [`DraftPromptBuilder::build`] turns one `(mail, account, trigger mode)`
//! tuple into a provider-agnostic [`ChatRequest`]:
//!
//! 1. **System block** — assistant identity for the account's display name,
//!    the role preamble from T074's [`assemble_role_context`], the semi-auto
//!    review note for E2/E3, and the money/deadline/legal-terms safety
//!    sentence (F_E1 §4.2).
//! 2. **Style block** — T076 [`build_style_block`] from the stored
//!    `account_ai_settings.style_profile`, appended to the system block; the
//!    cold-start fallback is reported through `BuiltPrompt::style_was_fallback`.
//! 3. **Context messages** — GTE snippets (≤ 3 × 200 chars, prepended), the
//!    latest 3 same-thread mails newest-first (bodies ≤ 500 chars), then the
//!    trigger mail (body ≤ 2 000 chars).
//! 4. **Task message** — the reply instruction, plus the user's optional
//!    extra instruction.
//!
//! Token budget (T079 §3): whitespace-word estimate × 1.3; the system+style
//! block is hard-capped at 500 tokens, the task at 100, the assembled total
//! must stay under `context_window × 0.90` or the call fails with
//! `AI_CONTEXT_TOO_LONG`. `knowledge_refs` (deduplicated source mail ids from
//! T074) pass through for the `ai_drafts`/`ai_decisions` audit columns.
//!
//! Log safety (09 §5): identifiers, counts, and token estimates only — never
//! prompt text, bodies, or the style block.

use uuid::Uuid;

use crate::ai::context::{assemble_role_context, RoleContextParams};
use crate::ai::style::{build_style_block, load_style_profile, StyleProfileJson};
use crate::ai::types::{Capability, ChatMessage, ChatRequest, ChatRole};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage::map_sqlx_err;
use crate::util::truncate_chars;

/// Reply drafts stay short on purpose (T079 §6).
pub const DRAFT_MAX_TOKENS: u32 = 512;
/// Draft generation temperature (F_E1 §4.2).
pub const DRAFT_TEMPERATURE: f32 = 0.3;

/// Hard cap on the system + style block (T079 §3).
const SYSTEM_TOKEN_CAP: usize = 500;
/// Hard cap on the task message (T079 §3).
const TASK_TOKEN_CAP: usize = 100;
/// Same-thread mails included in the prompt, newest first (F_E1 §4.2).
const THREAD_MAILS_IN_PROMPT: usize = 3;
/// Per-mail body cap for thread context messages.
const THREAD_BODY_PROMPT_CHARS: usize = 500;
/// Body cap for the trigger mail — newest and most complete (T079 §3).
const TRIGGER_BODY_PROMPT_CHARS: usize = 2_000;
/// GTE snippets injected, score-descending.
const GTE_SNIPPETS_MAX: usize = 3;
/// Per-snippet character cap.
const GTE_SNIPPET_CHARS: usize = 200;

const SEMI_AUTO_NOTE: &str =
    "This is a pre-generated draft for semi-auto mode. The user will review before sending.";
const SAFETY_NOTE: &str = "Never include commitments about money, deadlines, or legal terms \
unless they appear in the original email.";
const GTE_CONTEXT_PREFIX: &str = "Relevant context from past correspondence:";
const BASE_TASK: &str = "Please draft a professional and appropriate reply to the above email, \
in the same language as the original.";

/// Which E-mode triggered the generation. Maps 1:1 onto the
/// `ai_drafts.trigger_mode` column strings (dev/01 §ai_drafts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerMode {
    E1Manual,
    E2Semi,
    E3Auto,
}

impl TriggerMode {
    /// The persisted `trigger_mode` string.
    pub fn as_str(self) -> &'static str {
        match self {
            TriggerMode::E1Manual => "E1_manual",
            TriggerMode::E2Semi => "E2_semi",
            TriggerMode::E3Auto => "E3_auto",
        }
    }

    /// E2 and E3 both carry the review-context note (T079 §6: E3 keeps the
    /// same injection so generations stay consistent across modes).
    fn injects_review_note(self) -> bool {
        matches!(self, TriggerMode::E2Semi | TriggerMode::E3Auto)
    }
}

/// The assembled request plus the audit metadata the caller persists.
#[derive(Debug, Clone)]
pub struct BuiltPrompt {
    pub request: ChatRequest,
    /// Deduplicated source `mail_id`s of the GTE context (T074) — written to
    /// `ai_drafts.knowledge_refs` / `ai_decisions.knowledge_refs`.
    pub knowledge_refs: Vec<String>,
    /// `true` when the style block used the cold-start template (T076) — the
    /// E6 UI renders its "still learning" badge from this.
    pub style_was_fallback: bool,
}

/// Stateless assembly facade — all inputs travel through [`Self::build`].
pub struct DraftPromptBuilder;

impl DraftPromptBuilder {
    /// Assemble the full draft-generation prompt for one trigger mail. See the
    /// module docs for section order and budgets. Errors: `NOT_FOUND` (mail or
    /// account missing), `AI_CONTEXT_TOO_LONG`, plus the registry's resolution
    /// errors (`FORBIDDEN` when no provider is configured).
    pub async fn build(
        state: &AppState,
        mail_id: &str,
        account_id: &str,
        trigger_mode: TriggerMode,
        instruction: Option<&str>,
    ) -> AppResult<BuiltPrompt> {
        let db = state.storage.db().pool();

        // Account identity + role text (style fallback input).
        let account: Option<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT display_name, role_type, role_description FROM accounts WHERE id = ?",
        )
        .bind(account_id)
        .fetch_optional(db)
        .await
        .map_err(map_sqlx_err)?;
        let (display_name, role_type, role_description) = account.ok_or(AppError::NotFound)?;

        // Trigger mail's thread for same-thread context.
        let mail: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT thread_id FROM mails WHERE id = ? AND account_id = ? AND is_deleted = 0",
        )
        .bind(mail_id)
        .bind(account_id)
        .fetch_optional(db)
        .await
        .map_err(map_sqlx_err)?;
        let (thread_id,) = mail.ok_or(AppError::NotFound)?;

        // Provider resolution gives the model token window (dev/06 §5); the
        // request model comes from the account settings, like every E-call.
        let client = state.ai.resolve(account_id, Capability::DraftReply).await?;
        let context_window = client.context_window();
        let model = state
            .ai
            .account_config(account_id)
            .await?
            .model
            .unwrap_or_default();

        // Role preamble + GTE chunks + thread snippets (T074). The context
        // budget is the thread-context share of the window (T079 §3: ≤ 60 %).
        let mut params = RoleContextParams::new(
            mail_id,
            account_id,
            context_window * 60 / 100,
            Capability::DraftReply,
        );
        params.thread_id = thread_id;
        let ctx = assemble_role_context(state, &params).await?;

        // Style block (T076) from the stored profile; missing or unreadable
        // profiles fall back without blocking generation (AI_MODES §6.7).
        let profile: Option<StyleProfileJson> = load_style_profile(state.storage.db(), account_id)
            .await?
            .and_then(|v| serde_json::from_value(v).ok());
        let account_role = role_description
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .unwrap_or(&role_type)
            .to_string();
        let (style_block, style_was_fallback) = build_style_block(profile.as_ref(), &account_role);

        // ── System block (T079 §3 section 1 + 2) ────────────────────────────
        let mut system = format!(
            "You are a professional email assistant acting on behalf of {display_name}.\n{}",
            ctx.role_preamble
        );
        if trigger_mode.injects_review_note() {
            system.push('\n');
            system.push_str(SEMI_AUTO_NOTE);
        }
        system.push('\n');
        system.push_str(SAFETY_NOTE);
        system.push_str("\n\n");
        system.push_str(&style_block);
        let system = truncate_to_token_cap(&system, SYSTEM_TOKEN_CAP);

        // ── Context messages (T079 §3 section 3) ────────────────────────────
        let mut messages: Vec<ChatMessage> = Vec::new();
        let snippets: Vec<String> = ctx
            .chunks
            .iter()
            .take(GTE_SNIPPETS_MAX)
            .map(|c| truncate_chars(&c.snippet, GTE_SNIPPET_CHARS))
            .collect();
        if !snippets.is_empty() {
            messages.push(ChatMessage {
                role: ChatRole::User,
                content: format!("{GTE_CONTEXT_PREFIX}\n{}", snippets.join("\n---\n")),
            });
        }
        for mail in ctx.thread_mails.iter().take(THREAD_MAILS_IN_PROMPT) {
            messages.push(ChatMessage {
                role: ChatRole::User,
                content: format!(
                    "From: {}\nSubject: {}\n\n{}",
                    mail.from_email,
                    mail.subject,
                    truncate_chars(&mail.body, THREAD_BODY_PROMPT_CHARS)
                ),
            });
        }
        messages.push(ChatMessage {
            role: ChatRole::User,
            content: format!(
                "From: {}\nSubject: {}\n\n{}",
                ctx.target_mail.from_email,
                ctx.target_mail.subject,
                truncate_chars(&ctx.target_mail.body, TRIGGER_BODY_PROMPT_CHARS)
            ),
        });

        // ── Task message (T079 §3 section 4) ────────────────────────────────
        let mut task = BASE_TASK.to_string();
        if let Some(extra) = instruction.map(str::trim).filter(|i| !i.is_empty()) {
            task.push_str(" Additional instruction: ");
            task.push_str(extra);
        }
        messages.push(ChatMessage {
            role: ChatRole::User,
            content: truncate_to_token_cap(&task, TASK_TOKEN_CAP),
        });

        // ── Total budget gate (T079 §3): > window × 0.90 → context too long.
        let mut token_estimate = estimate_tokens_ws(&system);
        for message in &messages {
            token_estimate += estimate_tokens_ws(&message.content);
        }
        if token_estimate > context_window * 90 / 100 {
            tracing::warn!(
                event = "draft_prompt_over_budget",
                mail_id = %mail_id,
                account_id = %account_id,
                trigger_mode = trigger_mode.as_str(),
                token_estimate = token_estimate,
                context_window = context_window,
                "assembled draft prompt exceeds the model window"
            );
            return Err(AppError::AiContextTooLong);
        }

        let request = ChatRequest {
            model,
            system,
            messages,
            max_tokens: DRAFT_MAX_TOKENS,
            temperature: DRAFT_TEMPERATURE,
            stop: Vec::new(),
            purpose: Capability::DraftReply,
            request_id: Uuid::new_v4(),
        };

        // Identifiers and counts only — never prompt or style text (09 §5).
        tracing::info!(
            event = "draft_prompt_built",
            mail_id = %mail_id,
            account_id = %account_id,
            trigger_mode = trigger_mode.as_str(),
            token_estimate = token_estimate,
            knowledge_ref_count = ctx.knowledge_refs.len(),
            style_was_fallback = style_was_fallback,
            "draft prompt assembled"
        );

        Ok(BuiltPrompt {
            request,
            knowledge_refs: ctx.knowledge_refs,
            style_was_fallback,
        })
    }
}

/// Whitespace-word token estimate: 1 word ≈ 1.3 tokens, rounded up (T079 §3).
/// Empty text estimates to zero.
fn estimate_tokens_ws(s: &str) -> usize {
    let words = s.split_whitespace().count();
    (words * 13).div_ceil(10)
}

/// Hard-truncate `s` to roughly `cap` estimated tokens by keeping the leading
/// whitespace-separated words. Inner line breaks collapse to single spaces in
/// the truncated form — acceptable for an emergency cap that rarely fires.
fn truncate_to_token_cap(s: &str, cap: usize) -> String {
    if estimate_tokens_ws(s) <= cap {
        return s.to_string();
    }
    let max_words = cap * 10 / 13;
    s.split_whitespace()
        .take(max_words.max(1))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ai::mock::MockProvider;
    use crate::error::AppError;
    use crate::types::AiProvider;
    use crate::util::{new_uuid, now_unix, truncate_chars};
    use crate::vector::VectorRow;

    /// Account + ai-settings rows configured for the mock OpenAI provider.
    async fn seed_account(state: &AppState, role_description: Option<&str>) -> String {
        let id = new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
                 role_type, role_description, created_at, updated_at) \
             VALUES (?, ?, 'Maya Chen', 'imap', 'slate', 'W', 'work', ?, ?, ?)",
        )
        .bind(&id)
        .bind(format!("{id}@example.com"))
        .bind(role_description)
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO account_ai_settings (account_id, auth_level, ai_provider, ai_model, updated_at) \
             VALUES (?, 1, 'openai', 'gpt-4o', ?)",
        )
        .bind(&id)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        id
    }

    async fn seed_thread(state: &AppState, id: &str, account_id: &str) {
        sqlx::query(
            "INSERT INTO threads (id, account_id, subject, participants, latest_date, \
                 created_at, updated_at) VALUES (?, ?, 'Thread', '[]', 0, 0, 0)",
        )
        .bind(id)
        .bind(account_id)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    async fn seed_mail(
        state: &AppState,
        id: &str,
        account_id: &str,
        thread_id: Option<&str>,
        body: &str,
        date_sent: i64,
    ) {
        sqlx::query(
            "INSERT INTO mails (id, account_id, thread_id, message_id, subject, from_email, \
                 to_addrs, date_sent, date_received, body_text, snippet, embedding_status, \
                 created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, 'peer@vendorco.example', '[]', ?, ?, ?, ?, 'indexed', 0, 0)",
        )
        .bind(id)
        .bind(account_id)
        .bind(thread_id)
        .bind(format!("<{id}@x>"))
        .bind(format!("Subject {id}"))
        .bind(date_sent)
        .bind(date_sent)
        .bind(body)
        .bind(truncate_chars(body, 200))
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    /// Embed and index one mail so GTE retrieval can hit it.
    async fn index_mail(state: &AppState, id: &str, account_id: &str, text: &str) {
        let row = VectorRow {
            chunk_id: format!("{id}:0"),
            mail_id: id.into(),
            chunk_index: 0,
            account_id: account_id.into(),
            from_email: "peer@vendorco.example".into(),
            date_sent: now_unix(),
            subject: text.into(),
            snippet: text.into(),
            embedding_model: "bge-m3".into(),
            vector: state.embedder.embed(text).unwrap(),
        };
        state.storage.vectors().upsert(&[row]).unwrap();
    }

    fn register_mock(state: &AppState) -> Arc<MockProvider> {
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        state.ai.register(mock.clone());
        mock
    }

    const TRIGGER_BODY: &str =
        "the quarterly licensing contract renewal terms and the indemnity clause review";

    #[tokio::test]
    async fn e1_prompt_has_identity_role_and_task_but_no_review_note() {
        let (state, _rx) = AppState::test_state().await;
        register_mock(&state);
        let account = seed_account(&state, Some("Handle vendor correspondence.")).await;
        seed_mail(&state, "trigger", &account, None, TRIGGER_BODY, now_unix()).await;

        let built =
            DraftPromptBuilder::build(&state, "trigger", &account, TriggerMode::E1Manual, None)
                .await
                .unwrap();

        let system = &built.request.system;
        assert!(system.contains("acting on behalf of Maya Chen"));
        assert!(system.contains("Handle vendor correspondence."));
        assert!(system.contains("commitments about money, deadlines, or legal terms"));
        assert!(!system.contains("semi-auto mode"));
        assert!(!built.request.messages.is_empty());
        let last = built.request.messages.last().unwrap();
        assert!(last
            .content
            .contains("draft a professional and appropriate reply"));
        assert_eq!(built.request.max_tokens, DRAFT_MAX_TOKENS);
        assert_eq!(built.request.purpose, Capability::DraftReply);
        assert_eq!(built.request.model, "gpt-4o");
    }

    #[tokio::test]
    async fn e2_and_e3_prompts_carry_the_review_note() {
        let (state, _rx) = AppState::test_state().await;
        register_mock(&state);
        let account = seed_account(&state, None).await;
        seed_mail(&state, "trigger", &account, None, TRIGGER_BODY, now_unix()).await;

        for mode in [TriggerMode::E2Semi, TriggerMode::E3Auto] {
            let built = DraftPromptBuilder::build(&state, "trigger", &account, mode, None)
                .await
                .unwrap();
            assert!(
                built
                    .request
                    .system
                    .contains("pre-generated draft for semi-auto mode"),
                "{mode:?} must inject the review note"
            );
        }
    }

    #[tokio::test]
    async fn trigger_body_is_truncated_to_two_thousand_chars() {
        let (state, _rx) = AppState::test_state().await;
        register_mock(&state);
        let account = seed_account(&state, None).await;
        let huge = "clause ".repeat(1_500); // 10 500 chars
        seed_mail(&state, "trigger", &account, None, &huge, now_unix()).await;

        let built =
            DraftPromptBuilder::build(&state, "trigger", &account, TriggerMode::E1Manual, None)
                .await
                .unwrap();

        // Second-to-last message is the trigger mail; its body part is capped.
        let n = built.request.messages.len();
        let trigger_message = &built.request.messages[n - 2];
        let body_part = trigger_message.content.split("\n\n").nth(1).unwrap();
        assert_eq!(body_part.chars().count(), 2_000);
    }

    #[tokio::test]
    async fn thread_mails_capped_at_three_newest_first() {
        let (state, _rx) = AppState::test_state().await;
        register_mock(&state);
        let account = seed_account(&state, None).await;
        seed_thread(&state, "t1", &account).await;
        let base = now_unix();
        seed_mail(&state, "trigger", &account, Some("t1"), TRIGGER_BODY, base).await;
        for i in 0..5 {
            seed_mail(
                &state,
                &format!("tm{i}"),
                &account,
                Some("t1"),
                &format!("earlier reply number {i} in the thread"),
                base - 10 - i64::from(i),
            )
            .await;
        }

        let built =
            DraftPromptBuilder::build(&state, "trigger", &account, TriggerMode::E1Manual, None)
                .await
                .unwrap();

        let from_messages: Vec<&ChatMessage> = built
            .request
            .messages
            .iter()
            .filter(|m| m.content.starts_with("From: "))
            .collect();
        // 3 thread mails + the trigger mail.
        assert_eq!(from_messages.len(), 4);
        // Newest thread mails first: tm0 (newest) through tm2.
        assert!(from_messages[0].content.contains("earlier reply number 0"));
        assert!(from_messages[1].content.contains("earlier reply number 1"));
        assert!(from_messages[2].content.contains("earlier reply number 2"));
        assert!(from_messages[3]
            .content
            .contains("licensing contract renewal"));
    }

    #[tokio::test]
    async fn instruction_is_appended_to_the_task() {
        let (state, _rx) = AppState::test_state().await;
        register_mock(&state);
        let account = seed_account(&state, None).await;
        seed_mail(&state, "trigger", &account, None, TRIGGER_BODY, now_unix()).await;

        let built = DraftPromptBuilder::build(
            &state,
            "trigger",
            &account,
            TriggerMode::E1Manual,
            Some("Keep it under three sentences."),
        )
        .await
        .unwrap();
        let last = built.request.messages.last().unwrap();
        assert!(last
            .content
            .contains("Additional instruction: Keep it under three sentences."));
    }

    #[tokio::test]
    async fn style_fallback_flag_tracks_the_stored_profile() {
        let (state, _rx) = AppState::test_state().await;
        register_mock(&state);
        let account = seed_account(&state, Some("Negotiate vendor contracts.")).await;
        seed_mail(&state, "trigger", &account, None, TRIGGER_BODY, now_unix()).await;

        // No profile → cold-start fallback.
        let built =
            DraftPromptBuilder::build(&state, "trigger", &account, TriggerMode::E1Manual, None)
                .await
                .unwrap();
        assert!(built.style_was_fallback);
        assert!(built.request.system.contains("professional and courteous"));
        assert!(built.request.system.contains("Negotiate vendor contracts."));

        // Stored profile → learned style block, no fallback.
        let profile = serde_json::json!({
            "version": 1,
            "account_id": account,
            "generated_at": now_unix(),
            "summary": {
                "overall_tone": "Warm but direct; leads with the decision.",
                "opening_patterns": ["Hi {name},"],
                "closing_patterns": ["Best regards,"],
                "sentence_length": "12-18 words on average",
                "vocabulary": "Plain business English",
                "format_habit": "Short paragraphs."
            },
            "sample_snippets": [],
            "pinned": false
        });
        sqlx::query("UPDATE account_ai_settings SET style_profile = ? WHERE account_id = ?")
            .bind(profile.to_string())
            .bind(&account)
            .execute(state.storage.db().pool())
            .await
            .unwrap();

        let built =
            DraftPromptBuilder::build(&state, "trigger", &account, TriggerMode::E1Manual, None)
                .await
                .unwrap();
        assert!(!built.style_was_fallback);
        assert!(built
            .request
            .system
            .contains("Warm but direct; leads with the decision."));
    }

    #[tokio::test]
    async fn knowledge_refs_flow_from_gte_context() {
        let (state, _rx) = AppState::test_state().await;
        register_mock(&state);
        let account = seed_account(&state, None).await;
        seed_mail(&state, "trigger", &account, None, TRIGGER_BODY, now_unix()).await;
        let related = "prior contract renewal discussed the licensing terms and indemnity";
        seed_mail(&state, "k1", &account, None, related, now_unix() - 100).await;
        index_mail(&state, "k1", &account, related).await;

        let built =
            DraftPromptBuilder::build(&state, "trigger", &account, TriggerMode::E1Manual, None)
                .await
                .unwrap();
        assert!(built.knowledge_refs.contains(&"k1".to_string()));
        let gte_message = built
            .request
            .messages
            .iter()
            .find(|m| m.content.starts_with(GTE_CONTEXT_PREFIX));
        assert!(gte_message.is_some(), "GTE snippets must be prepended");
    }

    #[tokio::test]
    async fn tiny_context_window_is_context_too_long() {
        let (state, _rx) = AppState::test_state().await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai).with_context_window(40));
        state.ai.register(mock);
        let account = seed_account(&state, Some("Review inbound contracts for risk.")).await;
        seed_mail(&state, "trigger", &account, None, TRIGGER_BODY, now_unix()).await;

        let err =
            DraftPromptBuilder::build(&state, "trigger", &account, TriggerMode::E1Manual, None)
                .await
                .unwrap_err();
        assert!(matches!(err, AppError::AiContextTooLong));
    }

    #[tokio::test]
    async fn missing_mail_is_not_found() {
        let (state, _rx) = AppState::test_state().await;
        register_mock(&state);
        let account = seed_account(&state, None).await;

        let err =
            DraftPromptBuilder::build(&state, "missing", &account, TriggerMode::E1Manual, None)
                .await
                .unwrap_err();
        assert!(matches!(err, AppError::NotFound));
    }

    #[test]
    fn token_estimate_is_words_times_one_point_three() {
        let hundred_words = vec!["word"; 100].join(" ");
        assert_eq!(estimate_tokens_ws(&hundred_words), 130);
        assert_eq!(estimate_tokens_ws(""), 0);
        assert_eq!(estimate_tokens_ws("one"), 2); // ceil(1.3)
    }

    #[test]
    fn token_cap_truncation_keeps_leading_words() {
        let long = vec!["alpha"; 200].join(" "); // ~260 tokens
        let capped = truncate_to_token_cap(&long, 130);
        assert!(estimate_tokens_ws(&capped) <= 130);
        assert!(capped.starts_with("alpha"));
        // Under the cap → returned untouched.
        assert_eq!(truncate_to_token_cap("short text", 100), "short text");
    }

    #[test]
    fn trigger_mode_strings_match_the_schema() {
        assert_eq!(TriggerMode::E1Manual.as_str(), "E1_manual");
        assert_eq!(TriggerMode::E2Semi.as_str(), "E2_semi");
        assert_eq!(TriggerMode::E3Auto.as_str(), "E3_auto");
    }
}
