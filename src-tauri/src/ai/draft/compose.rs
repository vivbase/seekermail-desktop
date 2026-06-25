//! Compose-time AI Draft generation (analysis/57 §7).
//!
//! Unlike the E1/E2/E3 reply engine ([`super::engine`]), this is an
//! **ephemeral** helper. It builds a prompt from the user's intent + recipient
//! + (optional) forwarded excerpt, calls the configured provider once, and
//! returns the generated body text. Nothing is persisted — the user is already
//! in the compose window and sends manually, so there is no `ai_drafts` row and
//! no auto-send risk (the send path owns auditing).
//!
//! Modes ([`GenerateComposeDraftParams::mode`]):
//! * `"forward"` — a short forwarding note placed ABOVE the quoted message,
//!   shaped by an intent preset (handle / fyi / review / delegate / records).
//! * `"new"` — a complete short body from the user's free-text description.
//!
//! Reply / reply-all are intentionally NOT served here (analysis/57 D3): the
//! reading-view "AI Reply" path already owns that.
//!
//! Log safety (09 §5): identifiers and counts only — never prompt text or the
//! generated body.

use uuid::Uuid;

use crate::ai::draft::cleaner::clean_ai_body;
use crate::ai::provider::chat_with_retry;
use crate::ai::style::{build_style_block, load_style_profile, StyleProfileJson};
use crate::ai::types::{Capability, ChatMessage, ChatRequest, ChatRole};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage::map_sqlx_err;
use crate::types::{ComposeDraftResult, GenerateComposeDraftParams};
use crate::util::truncate_chars;

/// Compose copy can be a touch longer than a reply draft (a forward note, or a
/// whole short body), but stays bounded.
const COMPOSE_MAX_TOKENS: u32 = 600;
/// Slightly above reply drafts (0.3): compose copy benefits from a little more
/// variety while staying on task.
const COMPOSE_TEMPERATURE: f32 = 0.4;
/// Hard cap on the forwarded excerpt fed into the prompt (Unicode chars).
const EXCERPT_CHARS: usize = 1_200;

/// Generate an ephemeral compose body from the user's intent.
///
/// Errors: `NOT_FOUND` (account missing), `AI_PROVIDER_UNREACHABLE` (no
/// provider configured, or the provider is down), `AI_CONTEXT_TOO_LONG`,
/// `AI_RATE_LIMITED`. On any error nothing is returned and nothing is written.
pub async fn generate(
    state: &AppState,
    params: &GenerateComposeDraftParams,
) -> AppResult<ComposeDraftResult> {
    let account_id = params.account_id.as_str();
    let pool = state.storage.db().pool();

    // Account identity + role (the same slim lookup the draft prompt builder
    // uses). The role text becomes the signature dedup hint below.
    let account: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT display_name, role_type, role_description FROM accounts WHERE id = ?",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx_err)?;
    let (display_name, role_type, role_description) = account.ok_or(AppError::NotFound)?;
    let account_role = role_description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .unwrap_or(&role_type)
        .to_string();

    // Style block from the stored F_E5 profile; cold start falls back to a
    // generic-but-polite template, reported to the UI so it can show an "AI is
    // still learning your style" hint (AI_MODES §6.7).
    let profile: Option<StyleProfileJson> = load_style_profile(state.storage.db(), account_id)
        .await?
        .and_then(|v| serde_json::from_value(v).ok());
    let (style_block, style_was_fallback) = build_style_block(profile.as_ref(), &account_role);

    // Provider resolution gives the client + the configured model. DraftReply is
    // the writing capability; it is also the only bucket `chat_with_retry`
    // retries (drafts are safe to regenerate — nothing is sent yet).
    let client = state.ai.resolve(account_id, Capability::DraftReply).await?;
    let model = state
        .ai
        .account_config(account_id)
        .await?
        .model
        .unwrap_or_default();

    let system = format!(
        "You are a professional email assistant writing on behalf of {display_name}. \
Your role: {account_role}.\n{style_block}\n\
Write only the email body — no subject line, no preamble, no markdown fences. \
Do not invent facts, figures, amounts, dates, names, or commitments that are not provided to you."
    );

    let messages = vec![ChatMessage {
        role: ChatRole::User,
        content: build_task(params, &display_name),
    }];

    let request = ChatRequest {
        model,
        system,
        messages,
        max_tokens: COMPOSE_MAX_TOKENS,
        temperature: COMPOSE_TEMPERATURE,
        stop: Vec::new(),
        purpose: Capability::DraftReply,
        request_id: Uuid::new_v4(),
    };

    let response = chat_with_retry(client.as_ref(), request)
        .await
        .map_err(AppError::from)?;
    let body = clean_ai_body(&response.text, role_description.as_deref());

    Ok(ComposeDraftResult {
        body,
        style_was_fallback,
    })
}

/// Build the task instruction for the requested mode.
fn build_task(params: &GenerateComposeDraftParams, display_name: &str) -> String {
    let tone = params.tone.as_deref().unwrap_or("Friendly");
    let recipient = params
        .to
        .as_deref()
        .map(recipient_first_name)
        .unwrap_or_else(|| "the recipient".to_string());

    if params.mode == "new" {
        let about = params
            .note
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or(params.intent.as_deref())
            .unwrap_or("the matter at hand");
        return format!(
            "Write a complete, concise email body.\n\
Recipient: {recipient}.\n\
What the email is about: {about}.\n\
Tone: {tone}.\n\
Open with a brief greeting and sign off as {display_name}."
        );
    }

    // Default + "forward": a short cover note above the quoted message.
    let purpose = intent_instruction(params.intent.as_deref());
    let mut task = format!(
        "Write a brief forwarding note that will sit ABOVE the quoted message being forwarded.\n\
Recipient: {recipient}.\n\
{purpose}\n\
Tone: {tone}.\n"
    );
    if let Some(note) = params
        .note
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        task.push_str(&format!("Specific point to include: {note}.\n"));
    }
    if let Some(excerpt) = params
        .source_excerpt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let excerpt = truncate_chars(excerpt, EXCERPT_CHARS);
        task.push_str(&format!(
            "The message being forwarded, for context only — do not repeat it verbatim:\n---\n{excerpt}\n---\n"
        ));
    }
    task.push_str(&format!(
        "Write only the forwarding note: a short greeting, one to three sentences, and a sign-off as {display_name}."
    ));
    task
}

/// Map an intent preset id to a one-line instruction. Unknown ids pass through
/// verbatim so a future preset still produces a sane prompt.
fn intent_instruction(intent: Option<&str>) -> String {
    let line = match intent.unwrap_or("review") {
        "handle" => "Purpose: ask the recipient to take ownership and handle this directly.",
        "fyi" => {
            "Purpose: share this for the recipient's awareness; make clear no action is needed."
        }
        "review" => "Purpose: ask the recipient to review the details and advise before you reply.",
        "delegate" => {
            "Purpose: delegate the follow-up to the recipient and ask them to keep you informed."
        }
        "records" => {
            "Purpose: send this for the recipient's records; make clear no action is needed."
        }
        other => return format!("Purpose: {other}."),
    };
    line.to_string()
}

/// Best-effort first name from a `"Name <email>"` or bare-email recipient
/// string, for a natural greeting. Falls back to a capitalised local-part, then
/// to a neutral "there".
fn recipient_first_name(raw: &str) -> String {
    let raw = raw.trim();
    // "Name <email>" → the display name before '<'.
    if let Some(idx) = raw.find('<') {
        let name = raw[..idx].trim();
        if !name.is_empty() {
            return first_token(name);
        }
        // "<email>" with no display name → fall through to the local-part.
        let email = raw[idx..].trim_start_matches('<').trim_end_matches('>');
        return cap_local(email);
    }
    if raw.contains('@') {
        return cap_local(raw);
    }
    if raw.is_empty() {
        return "there".to_string();
    }
    first_token(raw)
}

fn first_token(s: &str) -> String {
    s.split_whitespace().next().unwrap_or(s).to_string()
}

fn cap_local(email: &str) -> String {
    let local = email.split('@').next().unwrap_or("");
    let mut chars = local.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "there".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::mock::MockProvider;
    use crate::ai::types::{ChatResponse, FinishReason, TokenUsage};
    use crate::types::AiProvider;
    use crate::util::{new_uuid, now_unix};
    use std::sync::Arc;

    /// Insert a minimal account + ai-settings row configured for `ai_provider`.
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

    fn ok_response(text: &str) -> Result<ChatResponse, crate::ai::ProviderError> {
        Ok(ChatResponse {
            text: text.to_string(),
            finish: FinishReason::Stop,
            usage: TokenUsage::default(),
            model_echo: "mock-model".into(),
            latency_ms: 1,
        })
    }

    fn forward_params(account_id: &str) -> GenerateComposeDraftParams {
        GenerateComposeDraftParams {
            account_id: account_id.to_string(),
            mode: "forward".into(),
            to: Some("Sarah Chen <s.chen@example.com>".into()),
            intent: Some("review".into()),
            note: Some("the payment terms".into()),
            tone: Some("Friendly".into()),
            source_excerpt: Some("Quotation: 1,200 units at $14.50 each, total $17,400.".into()),
        }
    }

    #[tokio::test]
    async fn forward_mode_returns_cleaned_body() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state, "openai").await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        mock.push_chat(ok_response(
            "Hi Sarah,\n\nForwarding the quotation below — could you review the payment terms \
             before I reply?\n\nThanks,\nMaya",
        ));
        state.ai.register(mock.clone());

        let result = generate(&state, &forward_params(&account)).await.unwrap();

        assert!(result.body.contains("Hi Sarah"));
        assert!(result.body.contains("payment terms"));
        // No stored style profile yet → the template fallback path.
        assert!(result.style_was_fallback);
        assert_eq!(mock.chat_call_count(), 1);
    }

    #[tokio::test]
    async fn new_mode_returns_body() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state, "openai").await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        mock.push_chat(ok_response(
            "Hi there,\n\nLet's set up a Q3 kickoff next week.\n\nThanks,\nMaya",
        ));
        state.ai.register(mock.clone());

        let params = GenerateComposeDraftParams {
            account_id: account.clone(),
            mode: "new".into(),
            to: None,
            intent: None,
            note: Some("schedule a Q3 kickoff".into()),
            tone: Some("Brief".into()),
            source_excerpt: None,
        };
        let result = generate(&state, &params).await.unwrap();
        assert!(result.body.contains("Q3 kickoff"));
        assert_eq!(mock.chat_call_count(), 1);
    }

    #[tokio::test]
    async fn unknown_account_is_not_found() {
        let (state, _rx) = AppState::test_state().await;
        let err = generate(&state, &forward_params("does-not-exist"))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound));
    }

    #[test]
    fn recipient_first_name_parses_display_and_email() {
        assert_eq!(recipient_first_name("Sarah Chen <s@x.com>"), "Sarah");
        assert_eq!(recipient_first_name("s.chen@example.com"), "S.chen");
        assert_eq!(recipient_first_name("<bob@x.com>"), "Bob");
        assert_eq!(recipient_first_name("   "), "there");
    }

    #[test]
    fn intent_instruction_covers_presets_and_passthrough() {
        assert!(intent_instruction(Some("handle")).contains("ownership"));
        assert!(intent_instruction(Some("custom thing")).contains("custom thing"));
        assert!(intent_instruction(None).contains("review"));
    }
}
