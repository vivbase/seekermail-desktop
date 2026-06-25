//! Thread-summary memory commands (P-4 wiring).
//!
//! The offline derivation of the memory layer ([`crate::ai::memory`]) is also
//! reachable on demand here, so it can be driven by a scheduled task or a
//! Settings action — the explicit complement to the automatic post-sync refresh.
//! Thin command wrapper per the command convention (03 §1): the logic lives in
//! `run_build` and the command maps `AppError → IpcError`.

use tauri::State;

use crate::account::AccountService;
use crate::ai::memory;
use crate::error::{AppResult, IpcError};
use crate::state::AppState;

/// Default thread cap per call when the caller doesn't specify one.
const DEFAULT_LIMIT: i64 = 50;

/// Build (or refresh) thread summaries for one account, or for all active
/// accounts when `account_id` is `None`. Returns how many summaries were
/// (re)built. Safe to schedule and repeat: provider-gated, bounded by the daily
/// query limit, and it only touches stale or unsummarised threads.
#[tauri::command]
pub async fn build_thread_summaries(
    state: State<'_, AppState>,
    account_id: Option<String>,
    limit: Option<i64>,
) -> Result<u32, IpcError> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 500);
    run_build(&state, account_id, limit)
        .await
        .map(|n| n as u32)
        .map_err(IpcError::from)
}

/// Resolve the target accounts and build summaries for each.
async fn run_build(state: &AppState, account_id: Option<String>, limit: i64) -> AppResult<usize> {
    let accounts: Vec<String> = match account_id {
        Some(id) => vec![id],
        None => AccountService::list(state)
            .await?
            .into_iter()
            .filter(|a| a.is_active)
            .map(|a| a.id)
            .collect(),
    };
    let mut total = 0usize;
    for id in accounts {
        let built = memory::build_thread_summaries(state, &id, limit).await?;
        // Refresh the level-2 digest when its inputs (the summaries) changed.
        if built > 0 {
            let _ = memory::build_inbox_digest(state, &id).await;
        }
        total += built;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ai::mock::MockProvider;
    use crate::ai::provider::ProviderError;
    use crate::ai::types::{AiProvider, ChatResponse, FinishReason, TokenUsage};

    async fn seed_account(state: &AppState, id: &str, provider: Option<&str>) {
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
                 role_type, role_description, is_active, is_primary, created_at, updated_at) \
             VALUES (?, ?, 'X', 'imap', 'slate', 'W', 'work', NULL, 1, 0, 0, 0)",
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

    async fn seed_thread(state: &AppState, id: &str, acc: &str) {
        sqlx::query(
            "INSERT INTO threads (id, account_id, subject, participants, mail_count, unread_count, \
                 latest_date, created_at, updated_at) \
             VALUES (?, ?, 'Renewal', '[]', 1, 0, 100, 0, 0)",
        )
        .bind(id)
        .bind(acc)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO mails (id, account_id, thread_id, message_id, subject, from_email, \
                 to_addrs, date_sent, date_received, body_text, snippet, embedding_status, \
                 created_at, updated_at) \
             VALUES (?, ?, ?, ?, 'Renewal', 'peer@x.com', '[]', 0, 0, 'renewal body', 'renewal', \
                 'indexed', 0, 0)",
        )
        .bind(format!("m-{id}"))
        .bind(acc)
        .bind(id)
        .bind(format!("<{id}@x>"))
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    fn ok_chat(text: &str) -> Result<ChatResponse, ProviderError> {
        Ok(ChatResponse {
            text: text.into(),
            finish: FinishReason::Stop,
            usage: TokenUsage {
                prompt_tokens: 40,
                completion_tokens: 10,
            },
            model_echo: "gpt-4o".into(),
            latency_ms: 50,
        })
    }

    #[tokio::test]
    async fn run_build_all_active_accounts() {
        let (state, _rx) = AppState::test_state().await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        mock.push_chat(ok_chat("{\"summary\":\"A.\",\"entities\":[]}"));
        mock.push_chat(ok_chat("{\"summary\":\"B.\",\"entities\":[]}"));
        state.ai.register(mock.clone());
        seed_account(&state, "a", Some("openai")).await;
        seed_account(&state, "b", Some("openai")).await;
        seed_thread(&state, "ta", "a").await;
        seed_thread(&state, "tb", "b").await;

        let built = run_build(&state, None, 50).await.unwrap();
        assert_eq!(built, 2, "one summary per account's stale thread");
    }

    #[tokio::test]
    async fn run_build_scopes_to_one_account() {
        let (state, _rx) = AppState::test_state().await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        mock.push_chat(ok_chat("{\"summary\":\"A.\",\"entities\":[]}"));
        state.ai.register(mock.clone());
        seed_account(&state, "a", Some("openai")).await;
        seed_account(&state, "b", Some("openai")).await;
        seed_thread(&state, "ta", "a").await;
        seed_thread(&state, "tb", "b").await;

        let built = run_build(&state, Some("a".into()), 50).await.unwrap();
        assert_eq!(built, 1, "only the named account is summarised");
    }
}
