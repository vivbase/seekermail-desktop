//! History backfill (T022).
//!
//! Knowledge-depth-driven, one-shot, resumable batch pull. Newest-first so recent
//! mail appears during the fill (F_A4 §3.2). Globally at most 2 accounts backfill
//! at once; batches of 200 with a 500 ms pause; pauses on low battery / low disk;
//! `last_uid_fetched` is the resume cursor.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use once_cell::sync::Lazy;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

use super::throttle;
use crate::config::{BACKFILL_BATCH_PAUSE_MS, BACKFILL_BATCH_SIZE, MAX_CONCURRENT_BACKFILLS};
use crate::error::AppResult;
use crate::state::AppState;
use crate::storage::{AccountRepo, BackfillRepo};
use crate::types::{ErrorCode, RawMail};
use crate::util::now_unix;

/// Global cap: at most 2 accounts backfilling at once (F_A4 §4.3).
static BACKFILL_SEM: Lazy<Arc<Semaphore>> =
    Lazy::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_BACKFILLS)));

/// Accounts with a backfill currently in flight. Guards against two starts for
/// one account racing on `backfill_state` — e.g. the first-sync trigger in
/// `poll_once` and the wizard's explicit `set_knowledge_depth` both firing for a
/// newly added mailbox. The second start collapses into a no-op.
static BACKFILL_INFLIGHT: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));

/// Seconds per "month" of knowledge depth (30-day months, F_A4 §3).
const MONTH_SECS: i64 = 30 * 86_400;

/// Start (or restart) a backfill for one account as a detached task.
pub fn spawn_start(state: AppState, account_id: String) -> JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(e) = run(&state, &account_id, false).await {
            tracing::warn!(account_id = %account_id, error = %e, "backfill failed");
            let _ = BackfillRepo::new(state.storage.db())
                .set_error(&account_id, &e.to_string())
                .await;
        }
    })
}

/// Resume any account whose backfill was interrupted (called at startup, T022 §3).
pub async fn resume_all(state: AppState) {
    let ids = match BackfillRepo::new(state.storage.db()).list_resumable().await {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!(error = %e, "backfill resume scan failed");
            return;
        }
    };
    for id in ids {
        let st = state.clone();
        tokio::spawn(async move {
            if let Err(e) = run(&st, &id, true).await {
                tracing::warn!(account_id = %id, error = %e, "backfill resume failed");
            }
        });
    }
}

/// Pause a running backfill (the loop notices the status on its next batch).
pub async fn pause(state: &AppState, account_id: &str) -> AppResult<()> {
    BackfillRepo::new(state.storage.db())
        .set_paused(account_id)
        .await
}

/// Resume a paused backfill.
pub async fn resume(state: AppState, account_id: String) -> AppResult<()> {
    spawn_start(state, account_id);
    Ok(())
}

async fn run(state: &AppState, account_id: &str, resume: bool) -> AppResult<()> {
    // Collapse duplicate concurrent starts for one account into a single run, so
    // the first-sync trigger and an explicit set_knowledge_depth can't race on
    // `backfill_state`. The guard clears on every exit path (incl. early returns).
    if !BACKFILL_INFLIGHT
        .lock()
        .expect("backfill inflight lock poisoned")
        .insert(account_id.to_string())
    {
        return Ok(());
    }
    let _inflight = InflightGuard(account_id.to_string());

    let _permit = BACKFILL_SEM
        .clone()
        .acquire_owned()
        .await
        .expect("backfill sem");
    state.backfill_active.store(true, Ordering::Relaxed);
    let _guard = scopeguard(state.clone());

    let repo = BackfillRepo::new(state.storage.db());
    let account = AccountRepo::new(state.storage.db()).get(account_id).await?;
    let depth = account.knowledge_depth_months;
    let boundary = depth
        .map(|m| now_unix() - (m as i64) * MONTH_SECS)
        .unwrap_or(0);

    // Connect + enumerate UIDs (newest first per the seam contract).
    let creds = super::sync::imap_creds_for(state, account_id).await?;
    let mut session = state.net.imap.open(creds).await?;
    let _ = session.select_inbox().await?;
    let mut uids = session.search_uids_since(boundary).await?;

    // Resume: drop UIDs at/above the cursor (we fill downward, newest→oldest).
    let resume_cursor = if resume {
        repo.get_opt(account_id)
            .await?
            .and_then(|s| s.last_uid_fetched)
    } else {
        None
    };
    if let Some(cursor) = resume_cursor {
        uids.retain(|&u| u < cursor);
    } else {
        repo.start(account_id, depth, Some(boundary), uids.len() as u32)
            .await?;
    }

    let total = repo
        .get_opt(account_id)
        .await?
        .and_then(|s| s.total_uid_count)
        .unwrap_or(uids.len() as u32);
    let mut fetched = repo
        .get_opt(account_id)
        .await?
        .map(|s| s.fetched_count)
        .unwrap_or(0);

    for batch in uids.chunks(BACKFILL_BATCH_SIZE) {
        // Pause if the user asked, or on low battery (F_A4 §4).
        if matches!(repo.get_opt(account_id).await?, Some(s) if s.status == "paused") {
            return Ok(());
        }
        if throttle::is_low_battery() {
            repo.set_paused(account_id).await?;
            state
                .events
                .sync_progress(account_id, fetched, Some(total), true);
            return Ok(());
        }
        // Stop on low disk.
        if state
            .storage
            .blobs()
            .check_free_space(&state.paths.root, 0)
            .is_err()
        {
            repo.set_paused(account_id).await?;
            state.events.sync_error(
                account_id,
                ErrorCode::FsDiskFull,
                "disk full — backfill paused",
            );
            return Ok(());
        }

        let bodies = session.fetch_bodies(batch).await?;
        let mut lowest = batch.iter().copied().min().unwrap_or(0);
        for (uid, bytes) in bodies {
            if state
                .mail_tx
                .send(RawMail {
                    account_id: account_id.to_string(),
                    folder: "INBOX".to_string(),
                    imap_uid: uid,
                    raw_bytes: bytes,
                })
                .await
                .is_err()
            {
                return Ok(()); // shutting down
            }
            fetched += 1;
            lowest = lowest.min(uid);
        }
        repo.advance(account_id, lowest, fetched).await?;
        state
            .events
            .sync_progress(account_id, fetched, Some(total), false);
        tokio::time::sleep(Duration::from_millis(BACKFILL_BATCH_PAUSE_MS)).await;
    }

    repo.set_completed(account_id).await?;
    Ok(())
}

/// Reset the global backfill-active flag when this task ends.
fn scopeguard(state: AppState) -> impl Drop {
    struct Guard(AppState);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.backfill_active.store(false, Ordering::Relaxed);
        }
    }
    Guard(state)
}

/// Clears an account's [`BACKFILL_INFLIGHT`] entry when its run ends (any path).
struct InflightGuard(String);
impl Drop for InflightGuard {
    fn drop(&mut self) {
        BACKFILL_INFLIGHT
            .lock()
            .expect("backfill inflight lock poisoned")
            .remove(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::fakes::{net_with_imap, FakeImapFactory, FakeMailbox};

    fn body(uid: i64) -> Vec<u8> {
        format!(
            "From: sender{uid}@example.com\r\nSubject: Message {uid}\r\n\
             Message-ID: <{uid}@example.com>\r\n\r\nBody {uid}\r\n"
        )
        .into_bytes()
    }

    async fn insert_imap_account(state: &AppState, id: &str) {
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, imap_host, color_token, \
                 badge_label, created_at, updated_at) \
             VALUES (?, 'a@x.com', 'A', 'imap', 'imap.example.com', 'slate', 'A', 0, 0)",
        )
        .bind(id)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    /// The history backfill is the sole importer of mail that already exists on the
    /// server (the incremental poll only fetches mail that arrives after the
    /// baseline). It must fetch every existing message and hand it to the ingest
    /// channel. With no knowledge depth set the boundary is 0 → import everything.
    #[tokio::test]
    async fn backfill_imports_existing_history_to_ingest_channel() {
        let account_id = "5f2d6a1e-0000-4000-8000-0000000000b1";
        let mailbox = FakeMailbox::new()
            .with_inbox(42, 100, 3)
            .with_uids([95, 96, 97])
            .with_body(95, body(95))
            .with_body(96, body(96))
            .with_body(97, body(97));
        let (state, mut rx) =
            AppState::test_state_with_net(net_with_imap(FakeImapFactory::new(mailbox))).await;
        insert_imap_account(&state, account_id).await;

        run(&state, account_id, false).await.unwrap();

        // Close the only sender so the receiver drains to completion.
        drop(state);
        let mut got: Vec<i64> = Vec::new();
        while let Some(raw) = rx.recv().await {
            got.push(raw.imap_uid);
        }
        got.sort_unstable();
        assert_eq!(
            got,
            vec![95, 96, 97],
            "backfill must import every existing message"
        );
    }
}
