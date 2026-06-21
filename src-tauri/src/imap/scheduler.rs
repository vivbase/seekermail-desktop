//! Sync scheduler (T021 §3, 03 §15).
//!
//! Owns one [`poll_task`](super::poll_task) per active account, a shared
//! `Semaphore(4)` capping global concurrency, and a `CancellationToken` for clean
//! shutdown. Also kicks off backfill resume at start. Managed by Tauri so the
//! `trigger_sync` command can poke an account on demand.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::{Notify, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::{backfill, idle_task, poll_task};
use crate::account::AccountService;
use crate::config::MAX_CONCURRENT_POLLS;
use crate::state::AppState;

struct AccountHandle {
    notify: Arc<Notify>,
    cancel: CancellationToken,
    poll_handle: JoinHandle<()>,
    idle_handle: JoinHandle<()>,
}

/// Background sync orchestrator.
pub struct SyncScheduler {
    state: AppState,
    sem: Arc<Semaphore>,
    root_cancel: CancellationToken,
    accounts: Mutex<HashMap<String, AccountHandle>>,
}

impl SyncScheduler {
    /// Start the scheduler: spawn poll tasks for every active account and resume
    /// any interrupted backfills.
    pub async fn start(state: AppState) -> Arc<Self> {
        let sched = Arc::new(Self {
            state: state.clone(),
            sem: Arc::new(Semaphore::new(MAX_CONCURRENT_POLLS)),
            root_cancel: CancellationToken::new(),
            accounts: Mutex::new(HashMap::new()),
        });

        if let Ok(accounts) = AccountService::list(&state).await {
            for a in accounts.into_iter().filter(|a| a.is_active) {
                sched.add_account(&a.id);
            }
        }

        // Resume backfills that were running/incomplete (T022 §3).
        backfill::resume_all(state).await;
        sched
    }

    /// Spawn a poll task for an account (idempotent — replaces any existing one).
    pub fn add_account(&self, account_id: &str) {
        let notify = Arc::new(Notify::new());
        let cancel = self.root_cancel.child_token();
        let poll_handle = poll_task::spawn(
            self.state.clone(),
            account_id.to_string(),
            self.sem.clone(),
            cancel.clone(),
            notify.clone(),
        );
        // The IDLE listener shares the poll task's `notify` (it pokes it the moment
        // new mail arrives) and `cancel` (one token stops both). It provides the
        // near-real-time push; the poll loop stays as the keepalive-interval safety
        // net and the fallback for servers that don't advertise IDLE.
        let idle_handle = idle_task::spawn(
            self.state.clone(),
            account_id.to_string(),
            cancel.clone(),
            notify.clone(),
        );
        let mut map = self.accounts.lock().expect("scheduler map poisoned");
        if let Some(old) = map.insert(
            account_id.to_string(),
            AccountHandle {
                notify,
                cancel,
                poll_handle,
                idle_handle,
            },
        ) {
            old.cancel.cancel();
        }
    }

    /// Stop and forget an account's poll task (on disable/delete).
    pub fn remove_account(&self, account_id: &str) {
        let removed = {
            let mut map = self.accounts.lock().expect("scheduler map poisoned");
            map.remove(account_id)
        };
        if let Some(h) = removed {
            h.cancel.cancel();
        }
    }

    /// Pause polling for one account (T053: reindex runs without sync churn).
    /// Implemented as task teardown — `resume_polling` re-spawns it.
    pub fn pause_polling(&self, account_id: &str) {
        self.remove_account(account_id);
        tracing::info!(
            event = "polling_paused",
            account_id = account_id,
            "poll task paused"
        );
    }

    /// Resume polling for one account after a pause (idempotent).
    pub fn resume_polling(&self, account_id: &str) {
        self.add_account(account_id);
        tracing::info!(
            event = "polling_resumed",
            account_id = account_id,
            "poll task resumed"
        );
    }

    /// Force an immediate poll for an account (the `trigger_sync` command).
    pub fn trigger_now(&self, account_id: &str) {
        let map = self.accounts.lock().expect("scheduler map poisoned");
        if let Some(h) = map.get(account_id) {
            h.notify.notify_one();
        }
    }

    /// Cancel every task and wait for them to finish (≤ a couple of seconds).
    pub async fn shutdown(&self) {
        self.root_cancel.cancel();
        let handles: Vec<JoinHandle<()>> = {
            let mut map = self.accounts.lock().expect("scheduler map poisoned");
            map.drain()
                .flat_map(|(_, h)| [h.poll_handle, h.idle_handle])
                .collect()
        };
        for h in handles {
            let _ = h.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn start_and_shutdown_clean() {
        let (state, _rx) = AppState::test_state().await;
        let sched = SyncScheduler::start(state).await;
        // No accounts → no tasks, shutdown returns promptly.
        sched.shutdown().await;
    }

    #[tokio::test]
    async fn add_then_remove_account() {
        let (state, _rx) = AppState::test_state().await;
        let sched = SyncScheduler::start(state).await;
        sched.add_account("acc-1");
        sched.trigger_now("acc-1"); // must not panic even with no real server
        sched.remove_account("acc-1");
        sched.shutdown().await;
    }
}
