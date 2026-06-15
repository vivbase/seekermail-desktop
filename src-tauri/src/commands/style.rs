//! Style-learning command (T075, 02 §Module E extension).
//!
//! `trigger_style_learning` is fire-and-forget: it validates that the account
//! has an `account_ai_settings` row, claims the per-account single-flight slot,
//! and returns immediately. Progress streams over `style:progress` /
//! `style:done` / `style:error` (see `crate::events`); a duplicate trigger
//! while a run is in flight is a no-op — the running task's event stream
//! already covers the caller.

use tauri::State;

use crate::ai::style;
use crate::error::{AppResult, IpcError};
use crate::state::AppState;

async fn do_trigger(state: &AppState, account_id: &str) -> AppResult<()> {
    // Fail fast with NOT_FOUND for unknown accounts; the heavy work is async.
    style::repo::load_style_profile(state.storage.db(), account_id).await?;
    style::trigger_style_learning_task(state.clone(), account_id.to_string());
    Ok(())
}

/// Start (or no-op into) a style-learning run for one account. Returns
/// immediately; results stream via `style:*` events. Errors: `NOT_FOUND`.
#[tauri::command]
pub async fn trigger_style_learning(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<(), IpcError> {
    do_trigger(&state, &account_id)
        .await
        .map_err(IpcError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ErrorCode;
    use crate::util::{new_uuid, now_unix};

    async fn seed_account(state: &AppState) -> String {
        let id = new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, created_at, updated_at) \
             VALUES (?, ?, 'Work', 'imap', 'slate', 'W', ?, ?)",
        )
        .bind(&id)
        .bind(format!("{id}@example.com"))
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO account_ai_settings (account_id, auth_level, updated_at) VALUES (?, 1, ?)",
        )
        .bind(&id)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        id
    }

    #[tokio::test]
    async fn trigger_unknown_account_is_not_found() {
        let (state, _rx) = AppState::test_state().await;
        let err = do_trigger(&state, "missing-account").await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn trigger_known_account_returns_immediately() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state).await;
        // No provider is configured — the spawned run will fail in the
        // background and surface as `style:error` (no-op emitter in tests);
        // the trigger itself must still return Ok at once.
        do_trigger(&state, &account).await.unwrap();
    }
}
