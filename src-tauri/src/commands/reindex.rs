//! Reindex commands (T053 §3b) — rebuild the GTE index from stored mails.
//!
//! Flow (F_H2 §4.3, data-intact mode): pause A4 polling + the live embed
//! worker → clear the account's vectors → reset `embedding_status` to
//! `pending` → rebuild FTS5 → drive embed batches synchronously with
//! checkpointing (`app_settings.gte.reindex_checkpoint_*`) → sample-verify →
//! resume everything. Cancellation keeps the checkpoint so a later run resumes.
//! `.eml` re-parse recovery mode is a later card.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use once_cell::sync::Lazy;
use tauri::State;

use crate::embedding::queue::{process_batch, EmbedJob, EMBED_BATCH_SIZE};
use crate::error::{AppError, AppResult, IpcError};
use crate::imap::SyncScheduler;
use crate::state::AppState;
use crate::storage::{map_sqlx_err, SettingRepo};
use crate::util::{new_uuid, now_unix};

/// Verification sample rate (F_H2 §4.3 — 5 % random sample).
const VERIFY_SAMPLE_RATE: f64 = 0.05;
/// Hard cap on sampled mails so verification stays sub-second.
const VERIFY_SAMPLE_MAX: i64 = 200;

/// Single-flight guard (09 §4 `GTE_REINDEX_IN_PROGRESS`).
static REINDEX_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Cancel flag for the active run.
static REINDEX_CANCEL: Lazy<Arc<AtomicBool>> = Lazy::new(|| Arc::new(AtomicBool::new(false)));

fn checkpoint_key(account_id: Option<&str>) -> String {
    format!("gte.reindex_checkpoint_{}", account_id.unwrap_or("all"))
}

/// The settings key the UI reads for the last completion report.
pub const REINDEX_REPORT_KEY: &str = "gte.last_reindex_report";

#[derive(serde::Serialize, serde::Deserialize)]
struct Checkpoint {
    processed_count: u64,
    started_at: i64,
}

#[allow(clippy::type_complexity)]
async fn pending_batch(
    state: &AppState,
    account_id: Option<&str>,
    limit: i64,
) -> AppResult<Vec<EmbedJob>> {
    let rows: Vec<(
        String,
        String,
        String,
        i64,
        String,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT id, account_id, from_email, date_sent, subject, snippet, body_text \
             FROM mails WHERE embedding_status = 'pending' AND is_deleted = 0 \
             AND (? IS NULL OR account_id = ?) \
             ORDER BY date_sent, id LIMIT ?",
    )
    .bind(account_id)
    .bind(account_id)
    .bind(limit)
    .fetch_all(state.storage.db().pool())
    .await
    .map_err(map_sqlx_err)?;
    Ok(rows
        .into_iter()
        .map(
            |(id, acc, from_email, date_sent, subject, snippet, body_text)| EmbedJob {
                mail_id: id,
                account_id: acc,
                from_email,
                date_sent,
                subject,
                snippet: snippet.unwrap_or_default(),
                body_text: body_text.unwrap_or_default(),
                retry: 0,
            },
        )
        .collect())
}

/// Reset statuses + clear vectors + rebuild FTS — the "fresh start" step. Only
/// runs when no checkpoint exists (a resumed run keeps prior progress).
async fn prepare_fresh_run(state: &AppState, account_id: Option<&str>) -> AppResult<u64> {
    let pool = state.storage.db().pool();

    // Spam (score > 0.8) is skipped by the vectorize policy (dev/04 §4 corpus rule).
    sqlx::query(
        "UPDATE mails SET embedding_status = 'pending' \
         WHERE is_deleted = 0 AND (? IS NULL OR account_id = ?) \
         AND (spam_score IS NULL OR spam_score <= 0.8)",
    )
    .bind(account_id)
    .bind(account_id)
    .execute(pool)
    .await
    .map_err(map_sqlx_err)?;

    match account_id {
        Some(id) => state.storage.vectors().delete_account(id)?,
        None => {
            state.storage.vectors().rebuild(std::iter::empty())?;
        }
    }

    // FTS5 external-content rebuild keeps the keyword index aligned with `mails`.
    sqlx::query("INSERT INTO mails_fts(mails_fts) VALUES('rebuild')")
        .execute(pool)
        .await
        .map_err(map_sqlx_err)?;

    let (total,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM mails WHERE embedding_status = 'pending' \
         AND is_deleted = 0 AND (? IS NULL OR account_id = ?)",
    )
    .bind(account_id)
    .bind(account_id)
    .fetch_one(pool)
    .await
    .map_err(map_sqlx_err)?;
    Ok(total.max(0) as u64)
}

/// Random-sample verification: every sampled mail marked `indexed` must have at
/// least one chunk in the vector store.
async fn verify_sample(state: &AppState, account_id: Option<&str>) -> AppResult<(u64, u64)> {
    let (indexed,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM mails WHERE embedding_status = 'indexed' \
         AND is_deleted = 0 AND (? IS NULL OR account_id = ?)",
    )
    .bind(account_id)
    .bind(account_id)
    .fetch_one(state.storage.db().pool())
    .await
    .map_err(map_sqlx_err)?;
    let sample_size = (((indexed as f64) * VERIFY_SAMPLE_RATE).ceil() as i64)
        .clamp(if indexed > 0 { 1 } else { 0 }, VERIFY_SAMPLE_MAX);
    if sample_size == 0 {
        return Ok((0, 0));
    }
    let ids: Vec<(String,)> = sqlx::query_as(
        "SELECT id FROM mails WHERE embedding_status = 'indexed' \
         AND is_deleted = 0 AND (? IS NULL OR account_id = ?) \
         ORDER BY RANDOM() LIMIT ?",
    )
    .bind(account_id)
    .bind(account_id)
    .bind(sample_size)
    .fetch_all(state.storage.db().pool())
    .await
    .map_err(map_sqlx_err)?;

    let mut verified: u64 = 0;
    let mut errors: u64 = 0;
    for (id,) in ids {
        if state.storage.vectors().contains_mail(&id) {
            verified += 1;
        } else {
            errors += 1;
        }
    }
    Ok((verified, errors))
}

async fn paused_accounts(state: &AppState, account_id: Option<&str>) -> Vec<String> {
    match account_id {
        Some(id) => vec![id.to_string()],
        None => sqlx::query_as::<_, (String,)>("SELECT id FROM accounts WHERE is_active = 1")
            .fetch_all(state.storage.db().pool())
            .await
            .map(|rows| rows.into_iter().map(|(id,)| id).collect())
            .unwrap_or_default(),
    }
}

/// The full reindex run. `Ok(true)` = completed, `Ok(false)` = cancelled.
async fn run_reindex(
    state: &AppState,
    account_id: Option<&str>,
    cancel: &AtomicBool,
) -> AppResult<bool> {
    let repo = SettingRepo::new(state.storage.db());
    let key = checkpoint_key(account_id);
    let started = now_unix();

    // Resume from a checkpoint when present; otherwise prepare a fresh run.
    let mut processed: u64 = match repo.get(&key).await? {
        Some(raw) => serde_json::from_str::<Checkpoint>(&raw)
            .map(|c| c.processed_count)
            .unwrap_or(0),
        None => {
            prepare_fresh_run(state, account_id).await?;
            repo.set(
                &key,
                &serde_json::to_string(&Checkpoint {
                    processed_count: 0,
                    started_at: started,
                })
                .expect("checkpoint serialises"),
            )
            .await?;
            0
        }
    };

    let (total,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM mails WHERE is_deleted = 0 AND (? IS NULL OR account_id = ?) \
         AND embedding_status IN ('pending', 'indexed', 'skipped')",
    )
    .bind(account_id)
    .bind(account_id)
    .fetch_one(state.storage.db().pool())
    .await
    .map_err(map_sqlx_err)?;
    let total = total.max(0) as u64;

    loop {
        if cancel.load(Ordering::SeqCst) {
            // Checkpoint stays → the next run resumes here (T053 §6).
            tracing::info!(
                event = "reindex_cancelled",
                count = processed,
                "reindex cancelled"
            );
            return Ok(false);
        }
        let batch = pending_batch(state, account_id, EMBED_BATCH_SIZE as i64).await?;
        if batch.is_empty() {
            break;
        }
        let batch_len = batch.len() as u64;
        process_batch(state, &batch).await;
        processed += batch_len;
        repo.set(
            &key,
            &serde_json::to_string(&Checkpoint {
                processed_count: processed,
                started_at: started,
            })
            .expect("checkpoint serialises"),
        )
        .await?;
        let rate = processed as f32 / ((now_unix() - started).max(1) as f32);
        state
            .events
            .gte_progress(processed, total.saturating_sub(processed), rate);
    }

    // Verification pass (stage "verify").
    let (verified, errors) = verify_sample(state, account_id).await?;
    let elapsed_ms = ((now_unix() - started).max(0) as u64) * 1000;
    let report = serde_json::json!({
        "processed": processed,
        "verifiedSample": verified,
        "verifyErrors": errors,
        "elapsedMs": elapsed_ms,
        "finishedAt": now_unix(),
    });
    repo.set(REINDEX_REPORT_KEY, &report.to_string()).await?;
    repo.delete(&key).await?;

    tracing::info!(
        event = "reindex_complete",
        count = processed,
        verified = verified,
        errors = errors,
        duration_ms = elapsed_ms,
        "reindex finished"
    );
    state.events.gte_finished(processed, elapsed_ms);
    Ok(true)
}

/// Spawn the reindex task; returns the task id. One run at a time.
pub async fn spawn_reindex(
    state: AppState,
    scheduler: Option<Arc<SyncScheduler>>,
    account_id: Option<String>,
) -> AppResult<String> {
    if REINDEX_ACTIVE.swap(true, Ordering::SeqCst) {
        return Err(AppError::GteReindexBusy);
    }
    REINDEX_CANCEL.store(false, Ordering::SeqCst);

    let task_id = new_uuid();
    tracing::info!(event = "reindex_started", task_id = %task_id, "reindex starting");

    let cancel = REINDEX_CANCEL.clone();
    tauri::async_runtime::spawn(async move {
        // Pause the live embed worker and A4 polling for the run (F_H2 §6).
        state.embed_queue.set_paused(true);
        let paused = paused_accounts(&state, account_id.as_deref()).await;
        if let Some(sched) = &scheduler {
            for id in &paused {
                sched.pause_polling(id);
            }
        }

        let result = run_reindex(&state, account_id.as_deref(), &cancel).await;

        // `finally`: always resume, even on error/cancel (T053 §6).
        state.embed_queue.set_paused(false);
        if let Some(sched) = &scheduler {
            for id in &paused {
                sched.resume_polling(id);
            }
        }
        REINDEX_ACTIVE.store(false, Ordering::SeqCst);

        if let Err(e) = result {
            tracing::warn!(
                event = "reindex_failed",
                code = e.code().as_wire(),
                "reindex failed"
            );
            state
                .events
                .gte_error("reindex", "index rebuild failed — see log");
        }
    });

    Ok(task_id)
}

/// Start rebuilding the GTE index (one account, or all when `None`).
#[tauri::command]
pub async fn start_reindex(
    state: State<'_, AppState>,
    scheduler: State<'_, Arc<SyncScheduler>>,
    account_id: Option<String>,
) -> Result<String, IpcError> {
    spawn_reindex((*state).clone(), Some((*scheduler).clone()), account_id)
        .await
        .map_err(IpcError::from)
}

/// Cancel the active reindex run; the checkpoint is kept for resume.
#[tauri::command]
pub async fn cancel_reindex(_task_id: String) -> Result<(), IpcError> {
    REINDEX_CANCEL.store(true, Ordering::SeqCst);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed(state: &AppState, account_id: &str, mails: usize) {
        let pool = state.storage.db().pool();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, created_at, updated_at) \
             VALUES (?, 'r@example.com', 'R', 'imap', 'slate', 'W', 0, 0)",
        )
        .bind(account_id)
        .execute(pool)
        .await
        .ok();
        for i in 0..mails {
            sqlx::query(
                "INSERT INTO mails (id, account_id, message_id, from_email, to_addrs, date_sent, \
                 date_received, subject, body_text, embedding_status, created_at, updated_at) \
                 VALUES (?, ?, ?, 's@x.y', '[]', ?, ?, 'Subject', 'meaningful body for chunks', 'indexed', 0, 0)",
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
    async fn full_run_indexes_writes_report_and_clears_checkpoint() {
        let (state, _rx) = AppState::test_state().await;
        seed(&state, "acc-r", 5).await;
        let cancel = AtomicBool::new(false);
        let done = run_reindex(&state, Some("acc-r"), &cancel).await.unwrap();
        assert!(done);

        // All mails indexed, vectors exist.
        let (pending,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM mails WHERE embedding_status = 'pending'")
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        assert_eq!(pending, 0);
        assert!(state.storage.vectors().stats().unwrap().total_vectors > 0);

        // Report written, checkpoint removed.
        let repo = SettingRepo::new(state.storage.db());
        let report = repo.get(REINDEX_REPORT_KEY).await.unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_str(&report).unwrap();
        assert_eq!(v["processed"], 5);
        assert_eq!(v["verifyErrors"], 0);
        assert!(repo
            .get(&checkpoint_key(Some("acc-r")))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn cancelled_run_keeps_checkpoint_for_resume() {
        let (state, _rx) = AppState::test_state().await;
        seed(&state, "acc-rc", 3).await;
        let cancel = AtomicBool::new(true); // cancel before the first batch
        let done = run_reindex(&state, Some("acc-rc"), &cancel).await.unwrap();
        assert!(!done);
        let repo = SettingRepo::new(state.storage.db());
        assert!(repo
            .get(&checkpoint_key(Some("acc-rc")))
            .await
            .unwrap()
            .is_some());

        // Resume completes and clears it.
        let cancel = AtomicBool::new(false);
        assert!(run_reindex(&state, Some("acc-rc"), &cancel).await.unwrap());
        assert!(repo
            .get(&checkpoint_key(Some("acc-rc")))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn second_start_while_active_is_busy() {
        let (state, _rx) = AppState::test_state().await;
        REINDEX_ACTIVE.store(true, Ordering::SeqCst);
        let err = spawn_reindex(state, None, None).await.unwrap_err();
        assert_eq!(err.code(), crate::types::ErrorCode::GteReindexInProgress);
        REINDEX_ACTIVE.store(false, Ordering::SeqCst);
    }
}
