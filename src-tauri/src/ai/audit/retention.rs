//! Audit-log retention sweep (T088 §3, F_E7 §4.3).
//!
//! `ai_decisions` is append-only through the [`super::logger`] API; this
//! module is the one sanctioned DELETE path — a daily policy purge of rows
//! older than `app_settings['ai.audit_retention_days']` (default 365 days,
//! `0` = keep forever). The sweep is best-effort: failures are `warn`-logged
//! and retried on the next tick.

use std::time::Duration;

use crate::error::AppResult;
use crate::state::AppState;
use crate::storage::SettingRepo;
use crate::util::now_unix;

/// `app_settings` key for the retention window in days.
pub const AUDIT_RETENTION_DAYS_KEY: &str = "ai.audit_retention_days";
/// Default retention when the setting is absent or unreadable.
pub const DEFAULT_AUDIT_RETENTION_DAYS: i64 = 365;
/// Sweep cadence: once per day (first run at startup).
pub const RETENTION_SWEEP_PERIOD_SECS: u64 = 86_400;

/// The configured retention window in days (`0` = keep forever).
async fn retention_days(state: &AppState) -> AppResult<i64> {
    let raw = SettingRepo::new(state.storage.db())
        .get(AUDIT_RETENTION_DAYS_KEY)
        .await?;
    Ok(raw
        .and_then(|v| serde_json::from_str::<i64>(&v).ok())
        .unwrap_or(DEFAULT_AUDIT_RETENTION_DAYS)
        .max(0))
}

/// Delete audit rows older than the retention window. Returns the number of
/// rows purged (`0` when retention is disabled).
pub async fn run_retention_sweep(state: &AppState) -> AppResult<u64> {
    let days = retention_days(state).await?;
    if days == 0 {
        return Ok(0);
    }
    let cutoff = now_unix() - days * 86_400;
    let result = sqlx::query("DELETE FROM ai_decisions WHERE created_at < ?")
        .bind(cutoff)
        .execute(state.storage.db().pool())
        .await
        .map_err(crate::storage::map_sqlx_err)?;
    let purged = result.rows_affected();
    if purged > 0 {
        tracing::info!(
            event = "audit_retention_purged",
            purged = purged,
            retention_days = days,
            "audit log retention sweep removed expired rows"
        );
    }
    Ok(purged)
}

/// Spawn the daily retention loop (called once from `lib.rs` at startup).
/// The first tick fires immediately so a long-closed app catches up.
pub fn start_retention_worker(state: AppState) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(RETENTION_SWEEP_PERIOD_SECS));
        loop {
            ticker.tick().await; // first tick fires immediately
            if let Err(e) = run_retention_sweep(&state).await {
                tracing::warn!(
                    event = "audit_retention_sweep_failed",
                    code = e.code().as_wire(),
                    "audit retention sweep failed; retrying next period"
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::new_uuid;

    async fn seed_account(state: &AppState) -> String {
        let id = new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
                 created_at, updated_at) VALUES (?, ?, 'Work', 'imap', 'slate', 'W', ?, ?)",
        )
        .bind(&id)
        .bind(format!("{id}@example.com"))
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        id
    }

    async fn seed_decision(state: &AppState, account_id: &str, created_at: i64) {
        sqlx::query(
            "INSERT INTO ai_decisions (id, account_id, decision_type, impact, \
                 action_description, result_description, created_at) \
             VALUES (?, ?, 'draft_created', 'reply', 'Recorded.', 'Stored.', ?)",
        )
        .bind(new_uuid())
        .bind(account_id)
        .bind(created_at)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    async fn decision_count(state: &AppState) -> i64 {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ai_decisions")
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        n
    }

    #[tokio::test]
    async fn sweep_purges_rows_past_the_default_window() {
        let (state, _rx) = crate::state::AppState::test_state().await;
        let account = seed_account(&state).await;
        let now = now_unix();
        seed_decision(&state, &account, now - 400 * 86_400).await; // past 365 d
        seed_decision(&state, &account, now - 10 * 86_400).await; // recent

        let purged = run_retention_sweep(&state).await.unwrap();
        assert_eq!(purged, 1);
        assert_eq!(decision_count(&state).await, 1);
    }

    #[tokio::test]
    async fn zero_retention_keeps_everything() {
        let (state, _rx) = crate::state::AppState::test_state().await;
        let account = seed_account(&state).await;
        SettingRepo::new(state.storage.db())
            .set(AUDIT_RETENTION_DAYS_KEY, "0")
            .await
            .unwrap();
        seed_decision(&state, &account, now_unix() - 1_000 * 86_400).await;

        let purged = run_retention_sweep(&state).await.unwrap();
        assert_eq!(purged, 0);
        assert_eq!(decision_count(&state).await, 1);
    }

    #[tokio::test]
    async fn custom_retention_window_is_honoured() {
        let (state, _rx) = crate::state::AppState::test_state().await;
        let account = seed_account(&state).await;
        SettingRepo::new(state.storage.db())
            .set(AUDIT_RETENTION_DAYS_KEY, "30")
            .await
            .unwrap();
        let now = now_unix();
        seed_decision(&state, &account, now - 40 * 86_400).await;
        seed_decision(&state, &account, now - 20 * 86_400).await;

        let purged = run_retention_sweep(&state).await.unwrap();
        assert_eq!(purged, 1);
        assert_eq!(decision_count(&state).await, 1);
    }
}
