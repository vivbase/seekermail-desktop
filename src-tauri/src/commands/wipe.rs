//! Wipe commands (T053 §3a) — destructive per-account data removal.
//!
//! `preview_wipe` returns the impact estimate the wizard shows before the
//! typed-`DELETE` confirmation; `start_wipe` runs the batched delete on Tokio,
//! streams `wipe:progress`, VACUUMs, and finishes with `wipe:complete
//! { freed_bytes }`. Guard rails: `Everything` may never remove the last
//! account (FORBIDDEN). Log fields are ids/counts only (09 §5).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::State;

use crate::error::{AppError, AppResult, IpcError};
use crate::imap::SyncScheduler;
use crate::state::AppState;
use crate::storage::map_sqlx_err;
use crate::types::{WipePreview, WipeScope};
use crate::util::new_uuid;

/// Rows deleted per batch (F_H2 §4.2).
pub const WIPE_BATCH_SIZE: i64 = 1000;

async fn do_preview_wipe(state: &AppState, account_ids: &[String]) -> AppResult<WipePreview> {
    if account_ids.is_empty() {
        return Err(AppError::Validation("select at least one account".into()));
    }
    let pool = state.storage.db().pool();
    let mut mail_count: u64 = 0;
    let mut attachment_count: u64 = 0;
    let mut estimated_bytes: u64 = 0;
    for id in account_ids {
        let (mails, bytes): (i64, Option<i64>) = sqlx::query_as(
            "SELECT COUNT(*), SUM(LENGTH(COALESCE(body_text,'')) + LENGTH(COALESCE(body_html,'')) + 1024) \
             FROM mails WHERE account_id = ?",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(map_sqlx_err)?;
        let (atts, att_bytes): (i64, Option<i64>) = sqlx::query_as(
            "SELECT COUNT(*), SUM(a.size_bytes) FROM attachments a \
             JOIN mails m ON m.id = a.mail_id WHERE m.account_id = ? AND a.downloaded = 1",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(map_sqlx_err)?;
        mail_count += mails.max(0) as u64;
        attachment_count += atts.max(0) as u64;
        estimated_bytes += (bytes.unwrap_or(0).max(0) + att_bytes.unwrap_or(0).max(0)) as u64;
    }
    Ok(WipePreview {
        mail_count,
        attachment_count,
        estimated_bytes,
    })
}

/// Last-account guard: `Everything` must leave ≥ 1 account configured.
async fn guard_not_last_account(state: &AppState, account_ids: &[String]) -> AppResult<()> {
    let (total,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM accounts")
        .fetch_one(state.storage.db().pool())
        .await
        .map_err(map_sqlx_err)?;
    if total <= account_ids.len() as i64 {
        return Err(AppError::Forbidden(
            "cannot wipe the configuration of the last remaining account".into(),
        ));
    }
    Ok(())
}

/// The batched delete loop for one account. Returns rows removed.
async fn wipe_account(
    state: &AppState,
    task_id: &str,
    account_id: &str,
    scope: WipeScope,
    progress: &mut u64,
    total: u64,
) -> AppResult<u64> {
    let pool = state.storage.db().pool();
    let mut removed: u64 = 0;
    loop {
        // attachments rows cascade from mails (FK ON DELETE CASCADE) and the FTS
        // triggers keep mails_fts in sync, so one batched mail delete is enough.
        let res = sqlx::query(
            "DELETE FROM mails WHERE id IN (SELECT id FROM mails WHERE account_id = ? LIMIT ?)",
        )
        .bind(account_id)
        .bind(WIPE_BATCH_SIZE)
        .execute(pool)
        .await
        .map_err(map_sqlx_err)?;
        let n = res.rows_affected();
        if n == 0 {
            break;
        }
        removed += n;
        *progress += n;
        state.events.wipe_progress(task_id, *progress, total);
    }

    // Thread aggregates for the account.
    sqlx::query("DELETE FROM threads WHERE account_id = ?")
        .bind(account_id)
        .execute(pool)
        .await
        .map_err(map_sqlx_err)?;

    if matches!(scope, WipeScope::MailsAndIndex | WipeScope::Everything) {
        // Derived vector index; the JSON-snapshot backend compacts on persist
        // (the LanceDB backend will expose an explicit compaction hook, T019).
        state.storage.vectors().delete_account(account_id)?;
    }

    if matches!(scope, WipeScope::Everything) {
        sqlx::query("DELETE FROM accounts WHERE id = ?")
            .bind(account_id)
            .execute(pool)
            .await
            .map_err(map_sqlx_err)?;
    }

    Ok(removed)
}

async fn run_wipe(
    state: &AppState,
    task_id: &str,
    account_ids: &[String],
    scope: WipeScope,
) -> AppResult<u64> {
    let preview = do_preview_wipe(state, account_ids).await?;
    let total = preview.mail_count;
    let mut progress: u64 = 0;

    // Blob bytes freed on disk (per-account directories, F_A3 §4.4).
    let mut freed_bytes: u64 = 0;
    for id in account_ids {
        wipe_account(state, task_id, id, scope, &mut progress, total).await?;
        freed_bytes += state
            .storage
            .blobs()
            .cleanup_account_dir(id)
            .await
            .unwrap_or(0);
    }

    // VACUUM reclaims SQLite pages; freed DB bytes measured by file size delta.
    let db_before = std::fs::metadata(&state.paths.db)
        .map(|m| m.len())
        .unwrap_or(0);
    sqlx::query("VACUUM")
        .execute(state.storage.db().pool())
        .await
        .map_err(map_sqlx_err)?;
    let db_after = std::fs::metadata(&state.paths.db)
        .map(|m| m.len())
        .unwrap_or(db_before);
    freed_bytes += db_before.saturating_sub(db_after);

    tracing::info!(
        event = "wipe_complete",
        task_id = task_id,
        count = progress,
        freed_bytes = freed_bytes,
        "wipe finished"
    );
    Ok(freed_bytes)
}

/// Spawn the wipe task; returns the task id immediately.
pub async fn spawn_wipe(
    state: AppState,
    scheduler: Option<Arc<SyncScheduler>>,
    account_ids: Vec<String>,
    scope: WipeScope,
) -> AppResult<String> {
    if account_ids.is_empty() {
        return Err(AppError::Validation("select at least one account".into()));
    }
    if matches!(scope, WipeScope::Everything) {
        guard_not_last_account(&state, &account_ids).await?;
    }

    let task_id = new_uuid();
    tracing::info!(
        event = "wipe_started",
        task_id = %task_id,
        account_count = account_ids.len(),
        scope = scope.as_wire(),
        "wipe task starting"
    );

    // Stop polling the affected accounts for the duration (and permanently for
    // `Everything`).
    if let Some(sched) = &scheduler {
        for id in &account_ids {
            sched.pause_polling(id);
        }
    }

    let tid = task_id.clone();
    tauri::async_runtime::spawn(async move {
        let result = run_wipe(&state, &tid, &account_ids, scope).await;
        // Resume polling unless the account config itself is gone.
        if let Some(sched) = &scheduler {
            if !matches!(scope, WipeScope::Everything) {
                for id in &account_ids {
                    sched.resume_polling(id);
                }
            }
        }
        match result {
            Ok(freed) => state.events.wipe_complete(&tid, freed),
            Err(e) => {
                tracing::warn!(event = "wipe_failed", task_id = %tid, code = e.code().as_wire(), "wipe failed");
                state.events.wipe_complete(&tid, 0);
            }
        }
    });

    Ok(task_id)
}

/// Cancellation flag shared with the (future) long-running wipe paths. v0.4
/// wipes are fast batched deletes; the flag exists so the IPC surface is stable.
static WIPE_CANCELLED: AtomicBool = AtomicBool::new(false);

/// Impact preview for the wipe wizard (step 2).
#[tauri::command]
pub async fn preview_wipe(
    state: State<'_, AppState>,
    account_ids: Vec<String>,
) -> Result<WipePreview, IpcError> {
    do_preview_wipe(&state, &account_ids)
        .await
        .map_err(IpcError::from)
}

/// Start the wipe; progress arrives via `wipe:*` events.
#[tauri::command]
pub async fn start_wipe(
    state: State<'_, AppState>,
    scheduler: State<'_, Arc<SyncScheduler>>,
    account_ids: Vec<String>,
    scope: WipeScope,
) -> Result<String, IpcError> {
    WIPE_CANCELLED.store(false, Ordering::SeqCst);
    spawn_wipe(
        (*state).clone(),
        Some((*scheduler).clone()),
        account_ids,
        scope,
    )
    .await
    .map_err(IpcError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed(state: &AppState, account_id: &str, mails: usize) {
        let pool = state.storage.db().pool();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, created_at, updated_at) \
             VALUES (?, ?, 'W', 'imap', 'slate', 'W', 0, 0)",
        )
        .bind(account_id)
        .bind(format!("{account_id}@example.com"))
        .execute(pool)
        .await
        .unwrap();
        for i in 0..mails {
            sqlx::query(
                "INSERT INTO mails (id, account_id, message_id, from_email, to_addrs, date_sent, \
                 date_received, body_text, created_at, updated_at) \
                 VALUES (?, ?, ?, 's@x.y', '[]', ?, ?, 'body text', 0, 0)",
            )
            .bind(format!("{account_id}-m{i}"))
            .bind(account_id)
            .bind(format!("<{account_id}-{i}@x>"))
            .bind(1000 + i as i64)
            .bind(1000 + i as i64)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn preview_counts_mails() {
        let (state, _rx) = AppState::test_state().await;
        seed(&state, "acc-w", 7).await;
        let p = do_preview_wipe(&state, &["acc-w".into()]).await.unwrap();
        assert_eq!(p.mail_count, 7);
        assert!(p.estimated_bytes > 0);
    }

    #[tokio::test]
    async fn wipe_removes_all_account_mails_and_vacuums() {
        let (state, _rx) = AppState::test_state().await;
        seed(&state, "acc-w2", 100).await;
        run_wipe(&state, "t-w", &["acc-w2".into()], WipeScope::MailsAndIndex)
            .await
            .unwrap();
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM mails WHERE account_id = 'acc-w2'")
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        assert_eq!(n, 0);
        // Account config survives (scope != Everything).
        let (a,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM accounts WHERE id = 'acc-w2'")
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        assert_eq!(a, 1);
    }

    #[tokio::test]
    async fn everything_on_last_account_is_forbidden() {
        let (state, _rx) = AppState::test_state().await;
        seed(&state, "only-acc", 1).await;
        let err = spawn_wipe(state, None, vec!["only-acc".into()], WipeScope::Everything)
            .await
            .unwrap_err();
        assert_eq!(err.code(), crate::types::ErrorCode::Forbidden);
    }

    #[tokio::test]
    async fn everything_removes_account_row_when_another_remains() {
        let (state, _rx) = AppState::test_state().await;
        seed(&state, "acc-a", 2).await;
        seed(&state, "acc-b", 2).await;
        run_wipe(&state, "t-e", &["acc-a".into()], WipeScope::Everything)
            .await
            .unwrap();
        let (a,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM accounts WHERE id = 'acc-a'")
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        assert_eq!(a, 0);
        let (b,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM accounts WHERE id = 'acc-b'")
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        assert_eq!(b, 1);
    }
}
