//! Sync-range commands (T053 §3c, F_A1 §4.5.4) — grow or shrink the per-account
//! local history window.
//!
//! The window is stored as `accounts.knowledge_depth_months` (the same field
//! the backfill planner consumes). **Grow** flags `sync_state.full_sync_required`
//! so the next scheduler pass backfills older history. **Shrink** deletes local
//! mails older than the new boundary (the wizard double-confirms with the
//! `preview_sync_range` count first).

use tauri::State;

use crate::error::{AppError, AppResult, IpcError};
use crate::imap::SyncScheduler;
use crate::state::AppState;
use crate::storage::map_sqlx_err;
use crate::types::SyncRangePreview;
use crate::util::now_unix;

/// Seconds per month for boundary math (30-day months, consistent with T016).
const SECS_PER_MONTH: i64 = 30 * 24 * 60 * 60;

fn boundary_unix(months: u32) -> i64 {
    now_unix() - (months as i64) * SECS_PER_MONTH
}

async fn current_depth(state: &AppState, account_id: &str) -> AppResult<Option<u32>> {
    let row: Option<(Option<i64>,)> =
        sqlx::query_as("SELECT knowledge_depth_months FROM accounts WHERE id = ?")
            .bind(account_id)
            .fetch_optional(state.storage.db().pool())
            .await
            .map_err(map_sqlx_err)?;
    match row {
        Some((depth,)) => Ok(depth.map(|d| d.max(0) as u32)),
        None => Err(AppError::NotFound),
    }
}

async fn do_preview(
    state: &AppState,
    account_id: &str,
    months: Option<u32>,
) -> AppResult<SyncRangePreview> {
    current_depth(state, account_id).await?; // 404 guard
    let beyond = match months {
        None => 0, // "all history" removes nothing
        Some(m) => {
            let (n,): (i64,) =
                sqlx::query_as("SELECT COUNT(*) FROM mails WHERE account_id = ? AND date_sent < ?")
                    .bind(account_id)
                    .bind(boundary_unix(m))
                    .fetch_one(state.storage.db().pool())
                    .await
                    .map_err(map_sqlx_err)?;
            n.max(0) as u64
        }
    };
    Ok(SyncRangePreview {
        mails_beyond_range: beyond,
    })
}

/// Apply the new range. Returns the number of local mails deleted (shrink only).
async fn do_update(
    state: &AppState,
    scheduler: Option<&SyncScheduler>,
    account_id: &str,
    months: Option<u32>,
) -> AppResult<u64> {
    let current = current_depth(state, account_id).await?;
    let pool = state.storage.db().pool();

    // Persist the new window.
    sqlx::query("UPDATE accounts SET knowledge_depth_months = ?, updated_at = ? WHERE id = ?")
        .bind(months.map(|m| m as i64))
        .bind(now_unix())
        .bind(account_id)
        .execute(pool)
        .await
        .map_err(map_sqlx_err)?;

    // Grow = new window reaches further back (or becomes unlimited).
    let grows = match (current, months) {
        (Some(_), None) => true,
        (Some(old), Some(new)) => new > old,
        (None, Some(_)) => false, // unlimited → bounded is a shrink
        (None, None) => false,
    };

    let mut deleted: u64 = 0;
    if grows {
        sqlx::query("UPDATE sync_state SET full_sync_required = 1 WHERE account_id = ?")
            .bind(account_id)
            .execute(pool)
            .await
            .map_err(map_sqlx_err)?;
        if let Some(sched) = scheduler {
            sched.trigger_now(account_id);
        }
    } else if let Some(m) = months {
        // Shrink: drop local rows beyond the new boundary (attachments cascade,
        // FTS triggers fire). The IMAP server copy is untouched.
        let res = sqlx::query("DELETE FROM mails WHERE account_id = ? AND date_sent < ?")
            .bind(account_id)
            .bind(boundary_unix(m))
            .execute(pool)
            .await
            .map_err(map_sqlx_err)?;
        deleted = res.rows_affected();
        // Index hygiene: vectors for removed mails go stale; account-level
        // delete keeps the derived index consistent (rebuilt lazily by B3).
        if deleted > 0 {
            state.storage.vectors().delete_account(account_id)?;
            sqlx::query(
                "UPDATE mails SET embedding_status = 'pending' \
                 WHERE account_id = ? AND embedding_status = 'indexed'",
            )
            .bind(account_id)
            .execute(pool)
            .await
            .map_err(map_sqlx_err)?;
        }
    }

    tracing::info!(
        event = "sync_range_updated",
        account_id = account_id,
        months = months.map(|m| m as i64).unwrap_or(-1),
        deleted = deleted,
        grew = grows,
        "sync range applied"
    );
    Ok(deleted)
}

/// How many local mails a shrink to `months` would delete.
#[tauri::command]
pub async fn preview_sync_range(
    state: State<'_, AppState>,
    account_id: String,
    months: Option<u32>,
) -> Result<SyncRangePreview, IpcError> {
    do_preview(&state, &account_id, months)
        .await
        .map_err(IpcError::from)
}

/// Apply a new sync range (`None` = all history). Returns deleted-row count.
#[tauri::command]
pub async fn update_sync_range(
    state: State<'_, AppState>,
    scheduler: State<'_, std::sync::Arc<SyncScheduler>>,
    account_id: String,
    months: Option<u32>,
) -> Result<u64, IpcError> {
    do_update(&state, Some(&**scheduler), &account_id, months)
        .await
        .map_err(IpcError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed(state: &AppState, account_id: &str, depth: Option<i64>) {
        let pool = state.storage.db().pool();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
             knowledge_depth_months, created_at, updated_at) \
             VALUES (?, 's@example.com', 'S', 'imap', 'slate', 'W', ?, 0, 0)",
        )
        .bind(account_id)
        .bind(depth)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO sync_state (account_id, updated_at) VALUES (?, 0)")
            .bind(account_id)
            .execute(pool)
            .await
            .ok();
        // One recent mail + one ancient mail.
        for (id, ts) in [
            ("new", now_unix() - 1000),
            ("old", now_unix() - 400 * 86400),
        ] {
            sqlx::query(
                "INSERT INTO mails (id, account_id, message_id, from_email, to_addrs, date_sent, \
                 date_received, created_at, updated_at) VALUES (?, ?, ?, 's@x.y', '[]', ?, ?, 0, 0)",
            )
            .bind(format!("{account_id}-{id}"))
            .bind(account_id)
            .bind(format!("<{account_id}-{id}@x>"))
            .bind(ts)
            .bind(ts)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn preview_counts_mails_beyond_boundary() {
        let (state, _rx) = AppState::test_state().await;
        seed(&state, "acc-s", Some(12)).await;
        let p = do_preview(&state, "acc-s", Some(3)).await.unwrap();
        assert_eq!(p.mails_beyond_range, 1); // only the 400-day-old mail
        let all = do_preview(&state, "acc-s", None).await.unwrap();
        assert_eq!(all.mails_beyond_range, 0);
    }

    #[tokio::test]
    async fn grow_sets_full_sync_required() {
        let (state, _rx) = AppState::test_state().await;
        seed(&state, "acc-g", Some(3)).await;
        let deleted = do_update(&state, None, "acc-g", Some(12)).await.unwrap();
        assert_eq!(deleted, 0);
        let (flag,): (i64,) =
            sqlx::query_as("SELECT full_sync_required FROM sync_state WHERE account_id = 'acc-g'")
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        assert_eq!(flag, 1);
    }

    #[tokio::test]
    async fn shrink_deletes_out_of_range_mails() {
        let (state, _rx) = AppState::test_state().await;
        seed(&state, "acc-h", Some(12)).await;
        let deleted = do_update(&state, None, "acc-h", Some(3)).await.unwrap();
        assert_eq!(deleted, 1);
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM mails WHERE account_id = 'acc-h'")
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        assert_eq!(n, 1); // the recent mail survives
    }

    #[tokio::test]
    async fn unknown_account_is_not_found() {
        let (state, _rx) = AppState::test_state().await;
        let err = do_preview(&state, "ghost", Some(3)).await.unwrap_err();
        assert_eq!(err.code(), crate::types::ErrorCode::NotFound);
    }
}
