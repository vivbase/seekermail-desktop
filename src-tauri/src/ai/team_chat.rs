//! Agent-IM (TEAM) channel intelligent reply — the "agent assistant" (F_I5).
//!
//! When a human posts a text message to the shared TEAM channel, one agent
//! answers in-channel. The reply is grounded in the operator's own mail: the
//! agent first runs a local GTE semantic search over its account, and any hit
//! above the score threshold is packed into the prompt so the agent can answer
//! questions like "are there any transaction-related emails?". With no relevant
//! hit it answers as a general assistant (the "both" behaviour: search when the
//! question is about mail, otherwise plain Q&A — decided by what the local index
//! returns, never a brittle keyword classifier). Only the final
//! [`AiProviderClient::chat`](super::provider::AiProviderClient::chat) call
//! leaves the device, to the BYO endpoint the user configured (ADR-0004: no
//! proxy).
//!
//! Routing: an `@DisplayName` mention picks that agent; otherwise the primary
//! account answers. The whole job runs detached from `post_im_message` so it
//! never blocks the command return (F_I2 §5), and only `sender_type == "human"`
//! messages trigger it, so an agent's own reply can never feed back into a loop.
//!
//! Capability routing reuses [`Capability::Summarize`] — a Team answer is a
//! summary/analysis over mail — so it flows through the same F4 provider matrix
//! and `daily_query_limit` guardrail as the rest of the AI subsystem. A
//! successful, model-generated reply writes a `team_reply` `ai_decisions` audit
//! row (identifiers, counts, and token figures only, 09 §5); the fallback notes
//! below are not AI decisions and stay unaudited.
//!
//! Log safety (09 §5): identifiers, counts, and categories only — never message
//! text, mail bodies, subjects, or addresses.

use serde_json::json;

use crate::account::AccountService;
use crate::ai::audit::{decision_type, AuditEntry};
use crate::ai::provider::ProviderError;
use crate::ai::types::{Capability, ChatMessage, ChatRequest, ChatRole};
use crate::error::AppError;
use crate::state::AppState;
use crate::storage::im_repo;
use crate::types::{Account, ImMessage, SearchResult, SemanticSearchParams};

/// How many of the most recent channel messages feed the conversation context.
const HISTORY_TURNS: i64 = 12;
/// How many semantic-search hits are packed into the prompt as mail context.
const SEARCH_HITS: u32 = 6;
/// Upper bound on the agent's reply length (tokens).
const REPLY_MAX_TOKENS: u32 = 800;
/// Chat temperature — a touch warmer than drafting (0.3), still grounded.
const REPLY_TEMPERATURE: f32 = 0.4;

/// Entry point from `post_im_message`: if `msg` is a non-empty human text
/// message, spawn a detached task that generates and posts one agent reply.
/// Returns immediately; never blocks the command (F_I2 §5).
pub fn spawn_reply(state: &AppState, msg: &ImMessage) {
    if msg.sender_type != "human" || msg.message_type != "text" {
        return;
    }
    let Some(user_text) = extract_text(&msg.content) else {
        return;
    };
    let user_text = user_text.trim().to_string();
    if user_text.is_empty() {
        return;
    }

    let state = state.clone();
    tauri::async_runtime::spawn(async move {
        generate_and_post(&state, &user_text).await;
    });
}

/// Generate one agent reply for `user_text` and post it to the shared channel.
/// Every failure mode (no agent, no provider, provider error) resolves to a
/// short in-channel note rather than silence, so the operator always gets a
/// reply they can act on.
async fn generate_and_post(state: &AppState, user_text: &str) {
    let Some(agent) = pick_responder(state, user_text).await else {
        // No account exists at all — nothing can answer (fresh-install edge).
        return;
    };

    // 1) Ground the answer in the operator's own mail (best-effort, local).
    let hits = search_mail(state, &agent.id, user_text).await;

    // 2) Resolve the BYO provider for this agent. A missing/unconfigured
    //    provider is the common first-run case — answer with a helpful nudge
    //    instead of staying silent.
    let client = match state.ai.resolve(&agent.id, Capability::Summarize).await {
        Ok(client) => client,
        Err(err) => {
            post_agent_text(
                state,
                &agent.id,
                &provider_unavailable_message(&agent, &err),
            )
            .await;
            return;
        }
    };

    // 3) Build the request: persona + (optional) mail context as the system
    //    preamble, recent turns as the conversation.
    let model = state
        .ai
        .account_config(&agent.id)
        .await
        .ok()
        .and_then(|config| config.model)
        .unwrap_or_default();
    let mut req = ChatRequest {
        model: model.clone(),
        system: build_system_prompt(&agent, &hits),
        messages: build_messages(state, user_text).await,
        max_tokens: REPLY_MAX_TOKENS,
        temperature: REPLY_TEMPERATURE,
        stop: Vec::new(),
        purpose: Capability::Summarize,
        request_id: uuid::Uuid::new_v4(),
    };
    // Honour the first-week conservative token cap when it is armed (T064).
    let _ = state.ai.clamp_chat_request(&mut req).await;

    // 4) One provider call. On success, capture the audit figures; any failure
    //    becomes a short in-channel note (not an AI decision, so not audited).
    let (reply, audit) = match client.chat(req).await {
        Ok(resp) => {
            let text = clean_reply(&resp.text);
            let model_used = if resp.model_echo.is_empty() {
                model.clone()
            } else {
                resp.model_echo.clone()
            };
            let audit = (!text.is_empty()).then_some((model_used, resp.usage, resp.latency_ms));
            (text, audit)
        }
        Err(err) => (provider_error_message(&err), None),
    };
    if reply.is_empty() {
        return;
    }

    // 5) Post the agent's reply to the shared channel.
    post_agent_text(state, &agent.id, &reply).await;

    // 6) Audit a model-generated reply (E7 coverage for the TEAM channel).
    //    Identifiers, counts, and token figures only — never message text or
    //    mail content (09 §5).
    if let Some((model_used, usage, latency_ms)) = audit {
        let entry = AuditEntry {
            account_id: agent.id.clone(),
            mail_id: None,
            draft_id: None,
            decision_type: decision_type::TEAM_REPLY.to_string(),
            impact: "context".into(),
            action_description: "Agent answered a message in the shared TEAM channel.".into(),
            result_description: format!(
                "Posted an in-channel reply grounded in {} matching email(s).",
                hits.len()
            ),
            knowledge_refs: hits.iter().map(|h| h.mail_id.clone()).collect(),
            knowledge_summary: None,
            ai_model: Some(model_used),
            input_tokens: Some(i64::from(usage.prompt_tokens)),
            output_tokens: Some(i64::from(usage.completion_tokens)),
            latency_ms: Some(i64::from(latency_ms)),
        };
        if let Err(err) = state.audit.log_await(entry).await {
            tracing::warn!(
                event = "team_reply_audit_failed",
                account_id = %agent.id,
                code = err.code().as_wire(),
                "failed to write the team-reply audit row"
            );
        }
    }

    tracing::info!(
        event = "team_reply_posted",
        account_id = %agent.id,
        grounded_in = hits.len(),
        "team agent reply posted"
    );
}

/// Pick the agent that answers: an `@DisplayName` mention wins, else the
/// primary active account, else the first active account, else any account.
async fn pick_responder(state: &AppState, user_text: &str) -> Option<Account> {
    let accounts = AccountService::list(state).await.ok()?;
    if accounts.is_empty() {
        return None;
    }

    // Mention routing: the composer inserts mentions as the literal text
    // "@DisplayName" (the name itself may contain spaces), so match the whole
    // "@name" by substring rather than tokenising on whitespace.
    let lower = user_text.to_lowercase();
    if lower.contains('@') {
        if let Some(mentioned) = accounts.iter().find(|account| {
            !account.display_name.trim().is_empty()
                && lower.contains(&format!("@{}", account.display_name.to_lowercase()))
        }) {
            return Some(mentioned.clone());
        }
    }

    if let Some(primary) = accounts.iter().find(|a| a.is_primary && a.is_active) {
        return Some(primary.clone());
    }
    if let Some(active) = accounts.iter().find(|a| a.is_active) {
        return Some(active.clone());
    }
    accounts.into_iter().next()
}

/// Best-effort GTE semantic search over the agent's own mailbox. Errors (no
/// index yet, embedder offline, …) degrade to "no context": the agent then
/// answers as a general assistant.
async fn search_mail(state: &AppState, account_id: &str, query: &str) -> Vec<SearchResult> {
    let params = SemanticSearchParams {
        query: query.to_string(),
        account_id: Some(account_id.to_string()),
        account_filter: None,
        date_from: None,
        date_to: None,
        // `None` → the 0.35 default, so only genuinely relevant mail is cited.
        min_score: None,
        limit: SEARCH_HITS,
        offset: 0,
    };
    match crate::search::ann::search_semantic(state, &params).await {
        Ok(page) => page.items,
        Err(err) => {
            tracing::debug!(
                event = "team_reply_search_skipped",
                error = %err,
                "semantic search unavailable for team reply; answering without mail context"
            );
            Vec::new()
        }
    }
}

/// Assemble the system preamble: the agent's persona, the behaviour rules, and
/// either the retrieved mail context or an explicit "no matches" note.
fn build_system_prompt(agent: &Account, hits: &[SearchResult]) -> String {
    let mut prompt = format!(
        "You are {name}, an AI assistant (a \"digital employee\") inside SeekerMail, a \
         local-first email client. You operate the mailbox {email}.\n",
        name = agent.display_name,
        email = agent.email,
    );
    if let Some(role) = agent
        .role_description
        .as_deref()
        .map(str::trim)
        .filter(|role| !role.is_empty())
    {
        prompt.push_str(&format!("Your role: {role}\n"));
    }
    prompt.push_str(
        "You are talking with your human operator in the shared TEAM channel.\n\
         Guidelines:\n\
         - Reply in the same language the operator used.\n\
         - Be concise, specific, and helpful.\n\
         - You may read and analyse the operator's mail, but never invent emails, senders, \
         dates, or facts.\n\
         - When relevant emails are listed below, ground your answer in them and cite the \
         sender, subject, and date. If they do not contain the answer, say so plainly.\n",
    );
    if hits.is_empty() {
        prompt.push_str(
            "\nNo emails from this mailbox matched the operator's message. Answer from the \
             conversation alone, and if the operator expected mail-based facts, tell them you \
             found no matching emails.\n",
        );
    } else {
        prompt.push_str(&format!(
            "\nRelevant emails from {email} (most relevant first):\n",
            email = agent.email
        ));
        for (idx, hit) in hits.iter().enumerate() {
            prompt.push_str(&format!("{}. {}\n", idx + 1, format_hit(hit)));
        }
    }
    prompt
}

/// One mail hit as a single grounding line: subject, sender, date, snippet.
fn format_hit(hit: &SearchResult) -> String {
    let sender = hit
        .from_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| format!("{name} <{}>", hit.from_email))
        .unwrap_or_else(|| hit.from_email.clone());
    let subject = {
        let trimmed = hit.subject.trim();
        if trimmed.is_empty() {
            "(no subject)"
        } else {
            trimmed
        }
    };
    format!(
        "\"{subject}\" — from {sender}, {date}. {snippet}",
        date = format_date(hit.date_sent),
        snippet = hit.snippet.trim(),
    )
}

/// Unix seconds → `YYYY-MM-DD` (UTC) for prompt citations.
fn format_date(unix: i64) -> String {
    use chrono::{TimeZone, Utc};
    match Utc.timestamp_opt(unix, 0).single() {
        Some(dt) => dt.format("%Y-%m-%d").to_string(),
        None => "unknown date".to_string(),
    }
}

/// Recent channel turns as a chat history, oldest first, ending on the
/// operator's current message. Only plain-text turns from the human and agents
/// become conversation; system notices and query cards are skipped.
async fn build_messages(state: &AppState, user_text: &str) -> Vec<ChatMessage> {
    let mut messages: Vec<ChatMessage> = recent_text_turns(state, HISTORY_TURNS)
        .await
        .into_iter()
        .filter_map(|msg| {
            let role = match msg.sender_type.as_str() {
                "human" => ChatRole::User,
                "agent" => ChatRole::Assistant,
                _ => return None,
            };
            let text = extract_text(&msg.content)?;
            let text = text.trim();
            (!text.is_empty()).then(|| ChatMessage {
                role,
                content: text.to_string(),
            })
        })
        .collect();

    // Guarantee the conversation ends on the operator's current question, even
    // if the history page raced the insert that triggered this reply.
    let ends_with_question = matches!(messages.last(), Some(last) if last.role == ChatRole::User && last.content == user_text);
    if !ends_with_question {
        messages.push(ChatMessage {
            role: ChatRole::User,
            content: user_text.to_string(),
        });
    }
    messages
}

/// The last `n` channel messages, oldest first. Two cheap reads: one to learn
/// the total, one to page the tail.
async fn recent_text_turns(state: &AppState, n: i64) -> Vec<ImMessage> {
    let db = state.storage.db();
    let total = match im_repo::list_messages(db, None, None, Some(1), Some(0)).await {
        Ok(page) => page.total as i64,
        Err(_) => return Vec::new(),
    };
    let offset = (total - n).max(0);
    im_repo::list_messages(db, None, None, Some(n), Some(offset))
        .await
        .map(|page| page.items)
        .unwrap_or_default()
}

/// Pull the human-readable text out of a channel message's JSON content
/// (`{"text": "..."}`). Returns `None` for cards that carry no `text` field;
/// falls back to the raw string if the content is not JSON at all.
fn extract_text(content: &str) -> Option<String> {
    match serde_json::from_str::<serde_json::Value>(content) {
        Ok(value) => value
            .get("text")
            .and_then(|text| text.as_str())
            .map(str::to_string),
        Err(_) => Some(content.to_string()),
    }
}

/// Insert one agent text message into the shared channel (best-effort).
async fn post_agent_text(state: &AppState, account_id: &str, text: &str) {
    let content = json!({ "text": text }).to_string();
    if let Err(err) = im_repo::insert_message(
        state.storage.db(),
        im_repo::MAIN_CHANNEL,
        "agent",
        account_id,
        "text",
        &content,
        None,
        None,
    )
    .await
    {
        tracing::warn!(
            event = "team_reply_post_failed",
            account_id = %account_id,
            error = %err,
            "failed to post agent reply to the team channel"
        );
    }
}

/// Trim the reply and strip a single wrapping ``` code fence if the model
/// wrapped the whole answer in one.
fn clean_reply(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        if let Some(end) = rest.rfind("```") {
            // Drop the optional language tag on the fence's first line.
            let inner = rest[..end]
                .split_once('\n')
                .map(|x| x.1)
                .unwrap_or(&rest[..end]);
            return inner.trim().to_string();
        }
    }
    trimmed.to_string()
}

/// In-channel note when no AI provider is configured/available for the agent.
/// The agent says so plainly and points to the fix — a real first-run answer,
/// never a silent drop.
fn provider_unavailable_message(agent: &Account, err: &AppError) -> String {
    match err {
        AppError::AiRateLimited => format!(
            "I've reached today's AI usage limit for {name}, so I can't answer right now. It \
             resets at midnight UTC — or raise the daily limit under Settings → AI Model.",
            name = agent.display_name,
        ),
        _ => format!(
            "I can't answer yet — no AI model is connected for {name} ({email}). Add one under \
             Settings → AI Model and I'll start replying here.",
            name = agent.display_name,
            email = agent.email,
        ),
    }
}

/// In-channel note when the provider call itself failed (network, auth, …).
fn provider_error_message(err: &ProviderError) -> String {
    match err {
        ProviderError::Unreachable(_) => {
            "I couldn't reach the AI provider just now. Please check your connection and try again."
        }
        ProviderError::Auth => {
            "The AI provider rejected its credentials. Please re-enter the API key under \
             Settings → AI Model."
        }
        ProviderError::RateLimited { .. } => {
            "The AI provider is rate-limiting requests right now. Please try again in a moment."
        }
        ProviderError::ContextTooLong => {
            "That was a bit too much context for the model — try a shorter question."
        }
        ProviderError::ContentFiltered => "The AI provider declined to answer that request.",
        ProviderError::BadResponse(_) | ProviderError::Canceled => {
            "Something went wrong talking to the AI provider. Please try again."
        }
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ai::mock::MockProvider;
    use crate::ai::types::{ChatResponse, FinishReason, TokenUsage};
    use crate::storage::im_repo;
    use crate::types::{AiProvider, ScoreLabel};
    use crate::util::{new_uuid, now_unix};

    /// Insert an account row (active; primary only when asked) and, optionally,
    /// an `account_ai_settings` row with the given provider.
    async fn seed_agent(
        state: &AppState,
        display_name: &str,
        primary: bool,
        ai_provider: Option<&str>,
    ) -> String {
        let id = new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
                 role_type, role_description, created_at, updated_at) \
             VALUES (?, ?, ?, 'imap', 'slate', 'W', 'work', \
                 'Coordinate vendor contracts and renewals.', ?, ?)",
        )
        .bind(&id)
        .bind(format!("{display_name}@example.com").to_lowercase())
        .bind(display_name)
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        sqlx::query("UPDATE accounts SET is_active = 1, is_primary = ? WHERE id = ?")
            .bind(i64::from(primary))
            .bind(&id)
            .execute(state.storage.db().pool())
            .await
            .unwrap();
        if let Some(provider) = ai_provider {
            sqlx::query(
                "INSERT INTO account_ai_settings (account_id, auth_level, ai_provider, ai_model, updated_at) \
                 VALUES (?, 1, ?, 'gpt-4o', ?)",
            )
            .bind(&id)
            .bind(provider)
            .bind(now)
            .execute(state.storage.db().pool())
            .await
            .unwrap();
        }
        id
    }

    async fn post_human(state: &AppState, text: &str) {
        let content = json!({ "text": text }).to_string();
        im_repo::insert_message(
            state.storage.db(),
            "main",
            "human",
            "human",
            "text",
            &content,
            None,
            None,
        )
        .await
        .unwrap();
    }

    async fn agent_replies(state: &AppState) -> Vec<String> {
        let all = im_repo::list_messages(state.storage.db(), None, None, Some(200), Some(0))
            .await
            .unwrap();
        all.items
            .into_iter()
            .filter(|m| m.sender_type == "agent" && m.message_type == "text")
            .filter_map(|m| extract_text(&m.content))
            .collect()
    }

    fn ok_response(text: &str) -> Result<ChatResponse, ProviderError> {
        Ok(ChatResponse {
            text: text.into(),
            finish: FinishReason::Stop,
            usage: TokenUsage {
                prompt_tokens: 40,
                completion_tokens: 20,
            },
            model_echo: "gpt-4o-2024-08-06".into(),
            latency_ms: 120,
        })
    }

    #[tokio::test]
    async fn happy_path_posts_agent_reply_with_model_text() {
        let (state, _rx) = AppState::test_state().await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        mock.push_chat(ok_response("No transaction emails in the last week."));
        state.ai.register(mock.clone());
        seed_agent(&state, "Agentboy", true, Some("openai")).await;

        post_human(&state, "Any transaction-related emails?").await;
        generate_and_post(&state, "Any transaction-related emails?").await;

        let replies = agent_replies(&state).await;
        assert_eq!(replies.len(), 1, "exactly one agent reply expected");
        assert!(replies[0].contains("No transaction emails"));
    }

    async fn team_reply_audit_count(state: &AppState) -> i64 {
        let (n,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM ai_decisions WHERE decision_type = 'team_reply'")
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        n
    }

    #[tokio::test]
    async fn successful_reply_writes_a_team_reply_audit_row() {
        let (state, _rx) = AppState::test_state().await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        mock.push_chat(ok_response("No transaction emails in the last week."));
        state.ai.register(mock.clone());
        seed_agent(&state, "Agentboy", true, Some("openai")).await;

        generate_and_post(&state, "Any transaction-related emails?").await;

        assert_eq!(team_reply_audit_count(&state).await, 1);
        let (decision_type, impact): (String, String) = sqlx::query_as(
            "SELECT decision_type, impact FROM ai_decisions WHERE decision_type = 'team_reply'",
        )
        .fetch_one(state.storage.db().pool())
        .await
        .unwrap();
        assert_eq!(decision_type, "team_reply");
        assert_eq!(impact, "context");
    }

    #[tokio::test]
    async fn fallback_note_writes_no_audit_row() {
        let (state, _rx) = AppState::test_state().await;
        // No provider configured → helpful fallback note, but no AI decision.
        state
            .ai
            .register(Arc::new(MockProvider::healthy(AiProvider::Openai)));
        seed_agent(&state, "Agentboy", true, None).await;

        generate_and_post(&state, "hello").await;

        assert_eq!(team_reply_audit_count(&state).await, 0);
    }

    #[tokio::test]
    async fn no_provider_posts_helpful_fallback_without_calling_a_provider() {
        let (state, _rx) = AppState::test_state().await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        state.ai.register(mock.clone());
        // No account_ai_settings row → resolve() returns NotFound.
        seed_agent(&state, "Agentboy", true, None).await;

        generate_and_post(&state, "hello").await;

        let replies = agent_replies(&state).await;
        assert_eq!(replies.len(), 1);
        assert!(
            replies[0].contains("no AI model is connected"),
            "got: {}",
            replies[0]
        );
        assert_eq!(mock.chat_call_count(), 0, "provider must not be called");
    }

    #[tokio::test]
    async fn provider_none_posts_fallback() {
        let (state, _rx) = AppState::test_state().await;
        seed_agent(&state, "Agentboy", true, Some("none")).await;

        generate_and_post(&state, "hello").await;

        let replies = agent_replies(&state).await;
        assert_eq!(replies.len(), 1);
        assert!(replies[0].contains("no AI model is connected"));
    }

    #[tokio::test]
    async fn provider_error_becomes_in_channel_note() {
        let (state, _rx) = AppState::test_state().await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        mock.set_default_chat_error(ProviderError::Unreachable("connect refused".into()));
        state.ai.register(mock.clone());
        seed_agent(&state, "Agentboy", true, Some("openai")).await;

        generate_and_post(&state, "summarise my inbox").await;

        let replies = agent_replies(&state).await;
        assert_eq!(replies.len(), 1);
        assert!(replies[0].contains("couldn't reach the AI provider"));
    }

    #[tokio::test]
    async fn mention_routes_to_the_named_agent() {
        let (state, _rx) = AppState::test_state().await;
        seed_agent(&state, "Agentboy", true, Some("openai")).await;
        let tiantian = seed_agent(&state, "Tiantian", false, Some("openai")).await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        mock.push_chat(ok_response("On it."));
        mock.push_chat(ok_response("On it."));
        state.ai.register(mock.clone());

        let resolved = pick_responder(&state, "@Tiantian can you check this?")
            .await
            .unwrap();
        assert_eq!(resolved.id, tiantian);
    }

    #[tokio::test]
    async fn no_mention_prefers_primary() {
        let (state, _rx) = AppState::test_state().await;
        let primary = seed_agent(&state, "Agentboy", true, Some("openai")).await;
        seed_agent(&state, "Tiantian", false, Some("openai")).await;

        let resolved = pick_responder(&state, "what's new?").await.unwrap();
        assert_eq!(resolved.id, primary);
    }

    #[tokio::test]
    async fn spawn_reply_ignores_non_human_and_empty() {
        // These must be no-ops (no panic, no task that would post a reply).
        let (state, _rx) = AppState::test_state().await;
        let agent_msg = ImMessage {
            id: "x".into(),
            channel_id: "main".into(),
            sender_type: "agent".into(),
            sender_id: "acc".into(),
            message_type: "text".into(),
            content: json!({ "text": "hi" }).to_string(),
            linked_email_id: None,
            status: "resolved".into(),
            created_at: now_unix(),
            read_at: None,
        };
        spawn_reply(&state, &agent_msg); // wrong sender_type → ignored

        let empty = ImMessage {
            content: json!({ "text": "   " }).to_string(),
            sender_type: "human".into(),
            ..agent_msg.clone()
        };
        spawn_reply(&state, &empty); // empty text → ignored
    }

    #[test]
    fn extract_text_handles_json_and_raw() {
        assert_eq!(
            extract_text(&json!({ "text": "hi" }).to_string()).as_deref(),
            Some("hi")
        );
        // A query card carries no top-level "text" → None.
        assert_eq!(
            extract_text(&json!({ "triggerType": "T1" }).to_string()),
            None
        );
        // Non-JSON falls back to the raw string.
        assert_eq!(extract_text("plain").as_deref(), Some("plain"));
    }

    #[test]
    fn build_system_prompt_includes_persona_and_hits() {
        let agent = Account {
            id: "a".into(),
            email: "alex@northwind.co".into(),
            display_name: "Alex".into(),
            provider: "imap".into(),
            imap_host: None,
            imap_port: 993,
            smtp_host: None,
            smtp_port: 587,
            color_token: "slate".into(),
            badge_label: "A".into(),
            role_type: "work".into(),
            role_description: Some("Coordinate vendor contracts.".into()),
            auth_level: 1,
            is_primary: true,
            is_active: true,
            sync_interval_secs: 300,
            last_synced_at: None,
            knowledge_depth_months: None,
            created_at: 0,
            updated_at: 0,
        };
        let hit = SearchResult {
            mail_id: "m1".into(),
            account_id: "a".into(),
            subject: "Invoice #42 payment".into(),
            from_name: Some("Vendor Co".into()),
            from_email: "billing@vendor.co".into(),
            date_sent: 1_700_000_000,
            snippet: "Your invoice is attached.".into(),
            score: 0.71,
            score_label: ScoreLabel::High,
            highlights: Vec::new(),
        };

        let with_hits = build_system_prompt(&agent, std::slice::from_ref(&hit));
        assert!(with_hits.contains("You are Alex"));
        assert!(with_hits.contains("Coordinate vendor contracts."));
        assert!(with_hits.contains("Invoice #42 payment"));
        assert!(with_hits.contains("billing@vendor.co"));
        assert!(with_hits.contains("2023-11-14")); // 1_700_000_000 → 2023-11-14 UTC

        let no_hits = build_system_prompt(&agent, &[]);
        assert!(no_hits.contains("No emails from this mailbox matched"));
    }

    #[test]
    fn clean_reply_strips_a_single_code_fence() {
        assert_eq!(clean_reply("  hello  "), "hello");
        assert_eq!(clean_reply("```\nhello\n```"), "hello");
        assert_eq!(clean_reply("```text\nhello\nworld\n```"), "hello\nworld");
        assert_eq!(clean_reply("no fence here"), "no fence here");
    }
}
