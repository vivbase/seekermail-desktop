//! Thread summariser — the *write* side of the P-4 memory derived layer
//! (analysis/54 §3.5).
//!
//! [`summarize_thread`] reads one thread's mails, asks the account's BYO provider
//! for a one-line gist + key entities, and stores it via
//! [`thread_summary_repo`](crate::storage::thread_summary_repo).
//! [`build_thread_summaries`] runs this over an account's stale/unsummarised
//! threads. This is meant to run **offline / on idle** (a later wiring step:
//! command + scheduler); the MCE read path never calls it — it only reads the
//! stored rows, which is what makes "summarise everything" cheap.
//!
//! Provider-gated like the rest of the AI subsystem (Module F): no provider →
//! nothing is built, never an error, so an idle/scheduled caller degrades
//! quietly.
//!
//! Log safety (09 §5): identifiers and counts only — never mail content or the
//! generated summary text.

use serde::Deserialize;
use sqlx::Row;

use crate::ai::types::{Capability, ChatMessage, ChatRequest, ChatRole};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage::inbox_digest_repo::{self, InboxDigestInput};
use crate::storage::thread_summary_repo::{self, ThreadSummary, ThreadSummaryInput};
use crate::util::{now_unix, truncate_chars};

/// Newest mails per thread fed to the summariser.
const THREAD_MAILS_FOR_SUMMARY: i64 = 8;
/// Per-mail body cap (chars) so one long mail can't dominate the prompt.
const SUMMARY_BODY_CHARS: usize = 500;
/// One-line summary cap (chars).
const SUMMARY_MAX_CHARS: usize = 200;
/// Max key-entity tags kept.
const MAX_ENTITIES: usize = 5;
/// Per-entity tag cap (chars).
const ENTITY_MAX_CHARS: usize = 40;
/// Output token ceiling for the summary call.
const SUMMARY_MAX_TOKENS: u32 = 200;
/// Low temperature — summaries should be faithful, not creative.
const SUMMARY_TEMPERATURE: f32 = 0.2;

const SUMMARY_SYSTEM: &str = "You summarise one email thread for a busy mailbox operator. \
Read the messages and reply with STRICT JSON only — no prose, no code fence: \
{\"summary\": \"one sentence (<=200 chars) capturing the gist and current status\", \
\"entities\": [\"up to 5 short tags: people, companies, or topics\"]}";

/// Recent thread summaries fed into the inbox digest (the level-2 input).
const DIGEST_SUMMARY_INPUT: i64 = 30;
/// Output token ceiling for the digest call.
const DIGEST_MAX_TOKENS: u32 = 300;
/// Inbox-digest cap (chars).
const DIGEST_MAX_CHARS: usize = 600;

const DIGEST_SYSTEM: &str = "You write a short inbox overview for a busy mailbox operator from \
one-line thread summaries and an unread count. Reply with 2–4 plain sentences (no JSON, no list, \
no code fence) capturing what needs attention and the overall state.";

/// Recent mails from a correspondent fed into the relationship note.
const ENRICH_MAILS: i64 = 6;
/// Per-mail body cap (chars) for the enrichment prompt.
const ENRICH_BODY_CHARS: usize = 300;
/// Relationship-note cap (chars).
const ENRICH_NOTE_CHARS: usize = 240;
/// Output token ceiling for the enrichment call.
const ENRICH_MAX_TOKENS: u32 = 120;
/// The `contacts.style_notes` JSON key the relationship note is merged under, so
/// it never clobbers the existing AI style profile (greeting/tone) that D2 reads.
const RELATIONSHIP_KEY: &str = "relationship";

const ENRICH_SYSTEM: &str = "In one plain sentence, note who this email correspondent is to the \
operator and what they typically discuss. No preamble, no JSON, no code fence.";

#[derive(Debug, Deserialize)]
struct SummaryJson {
    summary: String,
    #[serde(default)]
    entities: Vec<String>,
}

/// Summarise one thread and store the result; returns the stored summary.
/// `NotFound` when the thread is missing or not the account's; provider/network
/// failure surfaces as `AiUnreachable`.
pub async fn summarize_thread(
    state: &AppState,
    account_id: &str,
    thread_id: &str,
) -> AppResult<ThreadSummary> {
    let db = state.storage.db().pool();

    // Thread header (must belong to the account).
    let thread = sqlx::query(
        "SELECT subject, mail_count, latest_date FROM threads WHERE id = ? AND account_id = ?",
    )
    .bind(thread_id)
    .bind(account_id)
    .fetch_optional(db)
    .await
    .map_err(crate::storage::map_sqlx_err)?
    .ok_or(AppError::NotFound)?;
    let subject: String = thread.get("subject");
    let mail_count: i64 = thread.get("mail_count");
    let latest_date: i64 = thread.get("latest_date");

    // Newest mails in the thread, sanitised body only (B1).
    let mails = sqlx::query(
        "SELECT from_email, COALESCE(body_text, '') AS body \
         FROM mails WHERE thread_id = ? AND is_deleted = 0 \
         ORDER BY date_sent DESC LIMIT ?",
    )
    .bind(thread_id)
    .bind(THREAD_MAILS_FOR_SUMMARY)
    .fetch_all(db)
    .await
    .map_err(crate::storage::map_sqlx_err)?;

    let mut user = format!("Subject: {subject}\n\n");
    for m in &mails {
        let from: String = m.get("from_email");
        let body: String = m.get("body");
        user.push_str(&format!(
            "From: {from}\n{}\n---\n",
            truncate_chars(&body, SUMMARY_BODY_CHARS)
        ));
    }

    // Resolve the BYO provider and ask for the summary.
    let client = state.ai.resolve(account_id, Capability::Summarize).await?;
    let model = state
        .ai
        .account_config(account_id)
        .await
        .ok()
        .and_then(|c| c.model)
        .unwrap_or_default();
    let req = ChatRequest {
        model,
        system: SUMMARY_SYSTEM.to_string(),
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: user,
        }],
        max_tokens: SUMMARY_MAX_TOKENS,
        temperature: SUMMARY_TEMPERATURE,
        stop: Vec::new(),
        purpose: Capability::Summarize,
        request_id: uuid::Uuid::new_v4(),
    };
    let resp = client
        .chat(req)
        .await
        .map_err(|e| AppError::AiUnreachable(format!("thread summary failed: {e}")))?;

    let (summary, entities) = parse_summary(&resp.text, &subject);
    let model_used = (!resp.model_echo.is_empty()).then(|| resp.model_echo.clone());

    thread_summary_repo::upsert(
        state.storage.db(),
        &ThreadSummaryInput {
            thread_id: thread_id.to_string(),
            account_id: account_id.to_string(),
            summary,
            key_entities: entities,
            mail_count,
            latest_date,
            model: model_used,
        },
    )
    .await?;

    tracing::info!(
        event = "thread_summarised",
        account_id = %account_id,
        thread_id = %thread_id,
        mail_count,
        "stored thread summary"
    );

    thread_summary_repo::get(state.storage.db(), thread_id)
        .await?
        .ok_or(AppError::NotFound)
}

/// Threads summarised per sync — keeps any single sync's provider spend and
/// latency small; the rest catch up over subsequent syncs.
pub const AUTO_BUILD_PER_SYNC: i64 = 5;

/// Fire-and-forget: after a sync brings new mail, refresh a few stale/missing
/// thread summaries off the critical path. Provider-gated and bounded by the
/// account's `daily_query_limit` (the resolve guardrail), so it degrades quietly
/// with no provider and never blocks or slows the sync.
pub fn spawn_summary_build(state: &AppState, account_id: &str) {
    let state = state.clone();
    let account_id = account_id.to_string();
    tauri::async_runtime::spawn(async move {
        match build_thread_summaries(&state, &account_id, AUTO_BUILD_PER_SYNC).await {
            Ok(built) if built > 0 => {
                tracing::info!(
                    event = "auto_summary_build",
                    account_id = %account_id,
                    built,
                    "refreshed thread summaries after sync"
                );
                // Summaries changed → refresh the rolling inbox digest too.
                let _ = build_inbox_digest(&state, &account_id).await;
            }
            Ok(_) => {}
            Err(err) => tracing::debug!(
                event = "auto_summary_build_failed",
                account_id = %account_id,
                code = err.code().as_wire(),
                "background summary build failed"
            ),
        }
    });
}

/// Build summaries for an account's stale or unsummarised threads, newest first,
/// up to `limit`. Returns how many were (re)built. Provider/network failure
/// stops the batch early (those errors repeat per thread) and returns the count
/// done so far — never an error.
pub async fn build_thread_summaries(
    state: &AppState,
    account_id: &str,
    limit: i64,
) -> AppResult<usize> {
    let threads =
        thread_summary_repo::stale_or_missing_threads(state.storage.db(), account_id, limit)
            .await?;
    let mut built = 0usize;
    for thread_id in threads {
        match summarize_thread(state, account_id, &thread_id).await {
            Ok(_) => built += 1,
            Err(err) => {
                tracing::debug!(
                    event = "thread_summary_skipped",
                    account_id = %account_id,
                    thread_id = %thread_id,
                    code = err.code().as_wire(),
                    "stopping summary batch on error"
                );
                break;
            }
        }
    }
    tracing::info!(
        event = "thread_summaries_built",
        account_id = %account_id,
        built,
        "thread summary batch complete"
    );
    Ok(built)
}

/// Build (or refresh) the account's rolling inbox digest from its recent thread
/// summaries — the level-2 map-reduce reduction (analysis/54 §3.3/§3.5), so a
/// large mailbox's overview reads one paragraph instead of every thread summary.
/// Provider-gated. Returns `false` (built nothing) when there are no summaries to
/// reduce yet, no provider is configured, or the model returns nothing usable.
pub async fn build_inbox_digest(state: &AppState, account_id: &str) -> AppResult<bool> {
    let summaries =
        thread_summary_repo::list_recent(state.storage.db(), account_id, DIGEST_SUMMARY_INPUT)
            .await?;
    if summaries.is_empty() {
        return Ok(false);
    }
    let unread = unread_inbox_count(state, account_id).await?;

    let mut user = format!("Unread inbox mail: {unread}.\nRecent thread summaries:\n");
    for s in &summaries {
        user.push_str(&format!("- {}\n", s.summary.trim()));
    }

    let client = match state.ai.resolve(account_id, Capability::Summarize).await {
        Ok(client) => client,
        Err(_) => return Ok(false), // no provider → nothing built, quietly
    };
    let model = state
        .ai
        .account_config(account_id)
        .await
        .ok()
        .and_then(|c| c.model)
        .unwrap_or_default();
    let req = ChatRequest {
        model,
        system: DIGEST_SYSTEM.to_string(),
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: user,
        }],
        max_tokens: DIGEST_MAX_TOKENS,
        temperature: SUMMARY_TEMPERATURE,
        stop: Vec::new(),
        purpose: Capability::Summarize,
        request_id: uuid::Uuid::new_v4(),
    };
    let resp = client
        .chat(req)
        .await
        .map_err(|e| AppError::AiUnreachable(format!("inbox digest failed: {e}")))?;
    let digest = truncate_chars(strip_fence(&resp.text), DIGEST_MAX_CHARS);
    if digest.is_empty() {
        return Ok(false);
    }

    inbox_digest_repo::upsert(
        state.storage.db(),
        &InboxDigestInput {
            account_id: account_id.to_string(),
            digest,
            thread_count: summaries.len() as i64,
            unread_count: unread,
            model: (!resp.model_echo.is_empty()).then(|| resp.model_echo.clone()),
        },
    )
    .await?;

    tracing::info!(
        event = "inbox_digest_built",
        account_id = %account_id,
        threads = summaries.len(),
        "stored inbox digest"
    );
    Ok(true)
}

/// Enrich a contact with a one-line "who is this / what we discuss" note
/// (analysis/54 §3.5), merged under the `relationship` key of
/// `contacts.style_notes` so the existing AI style profile (greeting/tone, read
/// by D2) is preserved. Provider-gated; `false` when there is no such contact,
/// no mail from them, or no provider.
pub async fn enrich_contact(state: &AppState, account_id: &str, email: &str) -> AppResult<bool> {
    let email = email.trim().to_lowercase();
    let db = state.storage.db().pool();

    // The contact must exist (created during sync); read its current style JSON.
    let existing =
        sqlx::query_scalar::<_, Option<String>>("SELECT style_notes FROM contacts WHERE email = ?")
            .bind(&email)
            .fetch_optional(db)
            .await
            .map_err(crate::storage::map_sqlx_err)?;
    let Some(existing_notes) = existing else {
        return Ok(false); // no such contact
    };

    // Recent mail from this sender in this account.
    let mails = sqlx::query(
        "SELECT subject, COALESCE(body_text, '') AS body FROM mails \
         WHERE account_id = ? AND from_email = ? AND is_deleted = 0 \
         ORDER BY date_sent DESC LIMIT ?",
    )
    .bind(account_id)
    .bind(&email)
    .bind(ENRICH_MAILS)
    .fetch_all(db)
    .await
    .map_err(crate::storage::map_sqlx_err)?;
    if mails.is_empty() {
        return Ok(false);
    }

    let mut user = format!("Correspondent: {email}\nRecent messages:\n");
    for m in &mails {
        let subject: String = m.get("subject");
        let body: String = m.get("body");
        user.push_str(&format!(
            "- {subject}: {}\n",
            truncate_chars(&body, ENRICH_BODY_CHARS)
        ));
    }

    let client = match state.ai.resolve(account_id, Capability::Summarize).await {
        Ok(client) => client,
        Err(_) => return Ok(false),
    };
    let model = state
        .ai
        .account_config(account_id)
        .await
        .ok()
        .and_then(|c| c.model)
        .unwrap_or_default();
    let req = ChatRequest {
        model,
        system: ENRICH_SYSTEM.to_string(),
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: user,
        }],
        max_tokens: ENRICH_MAX_TOKENS,
        temperature: SUMMARY_TEMPERATURE,
        stop: Vec::new(),
        purpose: Capability::Summarize,
        request_id: uuid::Uuid::new_v4(),
    };
    let resp = client
        .chat(req)
        .await
        .map_err(|e| AppError::AiUnreachable(format!("contact enrich failed: {e}")))?;
    let note = truncate_chars(strip_fence(&resp.text), ENRICH_NOTE_CHARS);
    if note.is_empty() {
        return Ok(false);
    }

    // Merge the note under `relationship`, preserving any existing style JSON.
    let mut obj = existing_notes
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(s).ok())
        .unwrap_or_default();
    obj.insert(
        RELATIONSHIP_KEY.to_string(),
        serde_json::Value::String(note),
    );
    let merged = serde_json::Value::Object(obj).to_string();

    sqlx::query("UPDATE contacts SET style_notes = ?, updated_at = ? WHERE email = ?")
        .bind(&merged)
        .bind(now_unix())
        .bind(&email)
        .execute(db)
        .await
        .map_err(crate::storage::map_sqlx_err)?;

    tracing::info!(
        event = "contact_enriched",
        account_id = %account_id,
        "stored contact relationship note"
    );
    Ok(true)
}

/// Unread mail in the active inbox (received, not archived/deleted) — the digest
/// headline figure.
async fn unread_inbox_count(state: &AppState, account_id: &str) -> AppResult<i64> {
    let (n,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM mails WHERE account_id = ? \
         AND is_deleted = 0 AND is_archived = 0 AND is_sent = 0 AND is_read = 0",
    )
    .bind(account_id)
    .fetch_one(state.storage.db().pool())
    .await
    .map_err(crate::storage::map_sqlx_err)?;
    Ok(n)
}

/// Parse the model's JSON; tolerate a wrapping code fence and fall back to a
/// trimmed plain-text summary (then the subject) so a sloppy model never leaves
/// a thread without a usable one-liner.
fn parse_summary(text: &str, subject: &str) -> (String, Vec<String>) {
    let cleaned = strip_fence(text);
    let (summary, entities) = match serde_json::from_str::<SummaryJson>(cleaned) {
        Ok(j) => (j.summary, j.entities),
        Err(_) => (cleaned.to_string(), Vec::new()),
    };
    let mut summary = truncate_chars(summary.trim(), SUMMARY_MAX_CHARS);
    if summary.is_empty() {
        summary = truncate_chars(subject.trim(), SUMMARY_MAX_CHARS);
    }
    let entities = entities
        .into_iter()
        .filter_map(|e| {
            let e = e.trim();
            (!e.is_empty()).then(|| truncate_chars(e, ENTITY_MAX_CHARS))
        })
        .take(MAX_ENTITIES)
        .collect();
    (summary, entities)
}

/// Strip a single wrapping ``` fence (optionally ```json) if present.
fn strip_fence(text: &str) -> &str {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("```") {
        if let Some(end) = rest.rfind("```") {
            return rest[..end]
                .split_once('\n')
                .map(|x| x.1)
                .unwrap_or(&rest[..end])
                .trim();
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ai::mock::MockProvider;
    use crate::ai::provider::ProviderError;
    use crate::ai::types::{AiProvider, ChatResponse, FinishReason, TokenUsage};
    use crate::util::now_unix;

    async fn seed_account(state: &AppState, id: &str, provider: Option<&str>) {
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
                 role_type, role_description, is_active, created_at, updated_at) \
             VALUES (?, ?, 'X', 'imap', 'slate', 'W', 'work', NULL, 1, 0, 0)",
        )
        .bind(id)
        .bind(format!("{id}@x.com"))
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        if let Some(provider) = provider {
            sqlx::query(
                "INSERT INTO account_ai_settings (account_id, auth_level, ai_provider, ai_model, updated_at) \
                 VALUES (?, 1, ?, 'gpt-4o', 0)",
            )
            .bind(id)
            .bind(provider)
            .execute(state.storage.db().pool())
            .await
            .unwrap();
        }
    }

    async fn seed_thread(state: &AppState, id: &str, acc: &str, mail_count: i64, latest_date: i64) {
        sqlx::query(
            "INSERT INTO threads (id, account_id, subject, participants, mail_count, unread_count, \
                 latest_date, created_at, updated_at) \
             VALUES (?, ?, 'Quarterly renewal', '[]', ?, 0, ?, 0, 0)",
        )
        .bind(id)
        .bind(acc)
        .bind(mail_count)
        .bind(latest_date)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    async fn seed_mail(state: &AppState, id: &str, acc: &str, thread: &str, body: &str) {
        sqlx::query(
            "INSERT INTO mails (id, account_id, thread_id, message_id, subject, from_email, \
                 to_addrs, date_sent, date_received, body_text, snippet, embedding_status, \
                 created_at, updated_at) \
             VALUES (?, ?, ?, ?, 'Subject', 'peer@x.com', '[]', ?, ?, ?, ?, 'indexed', 0, 0)",
        )
        .bind(id)
        .bind(acc)
        .bind(thread)
        .bind(format!("<{id}@x>"))
        .bind(now_unix())
        .bind(now_unix())
        .bind(body)
        .bind(truncate_chars(body, 200))
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    fn ok_chat(text: &str) -> Result<ChatResponse, ProviderError> {
        Ok(ChatResponse {
            text: text.into(),
            finish: FinishReason::Stop,
            usage: TokenUsage {
                prompt_tokens: 50,
                completion_tokens: 20,
            },
            model_echo: "gpt-4o-2024-08-06".into(),
            latency_ms: 90,
        })
    }

    #[tokio::test]
    async fn summarize_thread_stores_parsed_summary() {
        let (state, _rx) = AppState::test_state().await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        mock.push_chat(ok_chat(
            "{\"summary\":\"Acme renewal pending sign-off.\",\"entities\":[\"Acme\",\"renewal\"]}",
        ));
        state.ai.register(mock.clone());
        seed_account(&state, "a", Some("openai")).await;
        seed_thread(&state, "t1", "a", 2, 100).await;
        seed_mail(
            &state,
            "m1",
            "a",
            "t1",
            "Please countersign the renewal contract.",
        )
        .await;

        let summary = summarize_thread(&state, "a", "t1").await.unwrap();
        assert_eq!(summary.summary, "Acme renewal pending sign-off.");
        assert_eq!(
            summary.key_entities,
            vec!["Acme".to_string(), "renewal".to_string()]
        );
        assert_eq!(summary.mail_count, 2);
        // Persisted and readable by the repo.
        let stored = thread_summary_repo::get(state.storage.db(), "t1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.summary, "Acme renewal pending sign-off.");
    }

    #[tokio::test]
    async fn summary_falls_back_to_plaintext_when_not_json() {
        let (state, _rx) = AppState::test_state().await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        mock.push_chat(ok_chat("Acme renewal is on track for next week."));
        state.ai.register(mock.clone());
        seed_account(&state, "a", Some("openai")).await;
        seed_thread(&state, "t1", "a", 1, 100).await;
        seed_mail(&state, "m1", "a", "t1", "renewal update").await;

        let summary = summarize_thread(&state, "a", "t1").await.unwrap();
        assert_eq!(summary.summary, "Acme renewal is on track for next week.");
        assert!(summary.key_entities.is_empty());
    }

    #[tokio::test]
    async fn build_summaries_covers_stale_and_missing() {
        let (state, _rx) = AppState::test_state().await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        mock.push_chat(ok_chat("{\"summary\":\"Thread one.\",\"entities\":[]}"));
        mock.push_chat(ok_chat("{\"summary\":\"Thread two.\",\"entities\":[]}"));
        state.ai.register(mock.clone());
        seed_account(&state, "a", Some("openai")).await;
        seed_thread(&state, "t1", "a", 1, 200).await;
        seed_thread(&state, "t2", "a", 1, 100).await;

        let built = build_thread_summaries(&state, "a", 10).await.unwrap();
        assert_eq!(built, 2);
        assert_eq!(
            thread_summary_repo::count(state.storage.db(), "a")
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn build_summaries_without_provider_builds_nothing() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "a", None).await; // no account_ai_settings
        seed_thread(&state, "t1", "a", 1, 100).await;

        let built = build_thread_summaries(&state, "a", 10).await.unwrap();
        assert_eq!(built, 0, "no provider → nothing built, no error");
    }

    async fn seed_summary(state: &AppState, thread: &str, acc: &str, summary: &str, latest: i64) {
        thread_summary_repo::upsert(
            state.storage.db(),
            &ThreadSummaryInput {
                thread_id: thread.into(),
                account_id: acc.into(),
                summary: summary.into(),
                key_entities: Vec::new(),
                mail_count: 1,
                latest_date: latest,
                model: None,
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn build_inbox_digest_reduces_summaries() {
        let (state, _rx) = AppState::test_state().await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        mock.push_chat(ok_chat("Two renewals await sign-off; the rest is routine."));
        state.ai.register(mock.clone());
        seed_account(&state, "a", Some("openai")).await;
        seed_thread(&state, "t1", "a", 1, 100).await;
        seed_thread(&state, "t2", "a", 1, 90).await;
        seed_summary(&state, "t1", "a", "Renewal A pending.", 100).await;
        seed_summary(&state, "t2", "a", "Renewal B pending.", 90).await;

        let built = build_inbox_digest(&state, "a").await.unwrap();
        assert!(built);
        let digest = inbox_digest_repo::get(state.storage.db(), "a")
            .await
            .unwrap()
            .expect("digest stored");
        assert_eq!(
            digest.digest,
            "Two renewals await sign-off; the rest is routine."
        );
        assert_eq!(digest.thread_count, 2, "reduced both summaries");
    }

    #[tokio::test]
    async fn build_inbox_digest_without_summaries_is_noop() {
        let (state, _rx) = AppState::test_state().await;
        state
            .ai
            .register(Arc::new(MockProvider::healthy(AiProvider::Openai)));
        seed_account(&state, "a", Some("openai")).await;

        let built = build_inbox_digest(&state, "a").await.unwrap();
        assert!(!built, "no thread summaries → nothing to reduce");
        assert!(inbox_digest_repo::get(state.storage.db(), "a")
            .await
            .unwrap()
            .is_none());
    }

    async fn seed_contact(state: &AppState, email: &str, style_notes: Option<&str>) {
        sqlx::query(
            "INSERT INTO contacts (id, email, first_seen_at, last_seen_at, interaction_count, \
                 reply_count, style_notes, created_at, updated_at) \
             VALUES (?, ?, 0, 0, 3, 1, ?, 0, 0)",
        )
        .bind(crate::util::new_uuid())
        .bind(email)
        .bind(style_notes)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    async fn seed_mail_from(
        state: &AppState,
        id: &str,
        acc: &str,
        from: &str,
        subject: &str,
        body: &str,
    ) {
        sqlx::query(
            "INSERT INTO mails (id, account_id, thread_id, message_id, subject, from_email, \
                 to_addrs, date_sent, date_received, body_text, snippet, embedding_status, \
                 created_at, updated_at) \
             VALUES (?, ?, NULL, ?, ?, ?, '[]', 0, 0, ?, ?, 'indexed', 0, 0)",
        )
        .bind(id)
        .bind(acc)
        .bind(format!("<{id}@x>"))
        .bind(subject)
        .bind(from)
        .bind(body)
        .bind(truncate_chars(body, 200))
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn enrich_contact_merges_relationship_note() {
        let (state, _rx) = AppState::test_state().await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        mock.push_chat(ok_chat(
            "Alex is the Acme account manager; you discuss renewals.",
        ));
        state.ai.register(mock.clone());
        seed_account(&state, "a", Some("openai")).await;
        seed_contact(&state, "alex@acme.com", Some("{\"greeting\":\"Hi Alex\"}")).await;
        seed_mail_from(
            &state,
            "m1",
            "a",
            "alex@acme.com",
            "Renewal terms",
            "Please review the renewal.",
        )
        .await;

        let ok = enrich_contact(&state, "a", "alex@acme.com").await.unwrap();
        assert!(ok);
        let notes: String =
            sqlx::query_scalar("SELECT style_notes FROM contacts WHERE email = 'alex@acme.com'")
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        let v: serde_json::Value = serde_json::from_str(&notes).unwrap();
        assert_eq!(
            v["relationship"],
            "Alex is the Acme account manager; you discuss renewals."
        );
        assert_eq!(v["greeting"], "Hi Alex", "existing style profile preserved");
    }

    #[tokio::test]
    async fn enrich_contact_missing_contact_is_noop() {
        let (state, _rx) = AppState::test_state().await;
        state
            .ai
            .register(Arc::new(MockProvider::healthy(AiProvider::Openai)));
        seed_account(&state, "a", Some("openai")).await;
        let ok = enrich_contact(&state, "a", "nobody@x.com").await.unwrap();
        assert!(!ok);
    }
}
