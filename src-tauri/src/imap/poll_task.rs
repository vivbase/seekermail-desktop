//! Per-account poll loop (T021 §3).
//!
//! Each enabled account owns one task that fires on its `sync_interval`, on an
//! explicit `trigger_now`, or exits on cancellation. It honours the backoff
//! watermark, skips overlapping ticks, and stops itself on an auth error.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Notify, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::{backoff, sync};
use crate::state::AppState;
use crate::storage::{AccountRepo, SyncStateRepo};
use crate::util::now_unix;

/// Spawn the loop for one account. The global `sem` caps total concurrency (4),
/// `cancel` stops the loop, `notify` forces an immediate poll.
pub fn spawn(
    state: AppState,
    account_id: String,
    sem: Arc<Semaphore>,
    cancel: CancellationToken,
    notify: Arc<Notify>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let interval_secs = AccountRepo::new(state.storage.db())
            .get(&account_id)
            .await
            .map(|a| a.sync_interval_secs.max(15))
            .unwrap_or(60);
        let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs as u64));

        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = ticker.tick() => {},
                _ = notify.notified() => {},
            }

            // Respect backoff / stopped state without taking a permit.
            if let Ok(ss) = SyncStateRepo::new(state.storage.db())
                .get(&account_id)
                .await
            {
                if ss.last_sync_result.as_deref() == Some("auth_error") {
                    break; // account is stopped until reauth (T018)
                }
                if matches!(ss.backoff_until, Some(until) if until > now_unix()) {
                    continue;
                }
            }

            let _permit = match sem.clone().acquire_owned().await {
                Ok(p) => p,
                Err(_) => break, // semaphore closed → shutting down
            };

            match sync::poll_once(&state, &account_id).await {
                Ok(_) => {
                    // Secondary folders (SENT / JUNK / TRASH) ride the same tick but
                    // are best-effort: a failure here must not trip the INBOX backoff
                    // or stop the loop. They don't need second-level latency, so the
                    // interval poll (not IDLE) is the right cadence.
                    if let Err(e) = sync::poll_secondary_folders(&state, &account_id).await {
                        tracing::warn!(
                            account_id = %account_id,
                            error = %e,
                            "secondary-folder sync failed (INBOX poll succeeded)"
                        );
                    }
                    // Safety net for queued write-backs (read/star → server); the
                    // per-action spawn_drain handles the prompt path.
                    if let Err(e) = super::outbound::drain(&state, &account_id).await {
                        tracing::warn!(account_id = %account_id, error = %e, "outbound drain failed");
                    }
                    // Inbound reconciliation (server→local read/star state + vanished
                    // messages); mirrors changes made from another device.
                    if let Err(e) = super::inbound::reconcile(&state, &account_id).await {
                        tracing::warn!(account_id = %account_id, error = %e, "inbound reconcile failed");
                    }
                }
                Err(e) if sync::is_auth_error(&e) => {
                    let _ = AccountRepo::new(state.storage.db())
                        .set_auth_failed(&account_id)
                        .await;
                    state
                        .events
                        .sync_error(&account_id, e.code(), "authentication failed");
                    break; // stop polling this account
                }
                Err(e) => {
                    let consec = SyncStateRepo::new(state.storage.db())
                        .get(&account_id)
                        .await
                        .map(|s| s.consecutive_errors)
                        .unwrap_or(0)
                        + 1;
                    let until = now_unix() + backoff::next_backoff(consec - 1).as_secs() as i64;
                    let _ = SyncStateRepo::new(state.storage.db())
                        .update_backoff(&account_id, consec, until)
                        .await;
                    state
                        .events
                        .sync_error(&account_id, e.code(), "sync failed");
                }
            }
        }
    })
}
