//! Per-account IMAP IDLE listener — push sync (complements the interval poll).
//!
//! It holds one long-lived connection parked in IMAP IDLE and, the moment the
//! server reports a mailbox change, pokes the *same* [`Notify`] the manual
//! `trigger_sync` uses — so every fetch still runs inside the one
//! [`poll_task`](super::poll_task) and two paths can never double-fetch the same
//! UID. On an IDLE timeout it re-issues IDLE (the RFC-2177 keepalive); on a
//! dropped connection it reconnects with capped backoff; on an auth error or
//! cancellation it exits.
//!
//! In the default (offline) build `open` simply fails, so the loop just backs off
//! — harmless. The real push path is wired under `--features live-net`.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::{backoff, sync};
use crate::config::IDLE_KEEPALIVE_SECS;
use crate::net::IdleOutcome;
use crate::state::AppState;
use crate::storage::{AccountRepo, SyncStateRepo};

/// How a single IDLE *connection* ended.
enum ConnEnd {
    /// Stop the whole listener (cancelled, auth-stopped, or unconfigured account).
    Stop,
    /// The connection dropped transiently — reconnect after a backoff.
    Reconnect,
}

/// Spawn the IDLE listener for one account. `cancel` is shared with the account's
/// poll task (one cancel stops both); `notify` is that poll task's wake handle.
pub fn spawn(
    state: AppState,
    account_id: String,
    cancel: CancellationToken,
    notify: Arc<Notify>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut consecutive_errors: u32 = 0;
        loop {
            if cancel.is_cancelled() {
                break;
            }
            // Honour the account's stopped (auth_error) state — same rule the poll
            // task applies, so a de-authed account never keeps a socket open.
            if let Ok(ss) = SyncStateRepo::new(state.storage.db())
                .get(&account_id)
                .await
            {
                if ss.last_sync_result.as_deref() == Some("auth_error") {
                    break;
                }
            }

            match run_one_connection(
                &state,
                &account_id,
                &cancel,
                &notify,
                &mut consecutive_errors,
            )
            .await
            {
                ConnEnd::Stop => break,
                ConnEnd::Reconnect => {
                    consecutive_errors += 1;
                    let wait = backoff::next_backoff(consecutive_errors - 1);
                    if sleep_or_cancelled(&cancel, wait).await {
                        break;
                    }
                    continue;
                }
            }
        }
    })
}

/// Open one IDLE connection and run it until it ends.
async fn run_one_connection(
    state: &AppState,
    account_id: &str,
    cancel: &CancellationToken,
    notify: &Arc<Notify>,
    consecutive_errors: &mut u32,
) -> ConnEnd {
    // Build credentials. A missing host / deleted account is terminal; stop rather
    // than spin. (OAuth pre-refresh is the poll path's job — a stale token here
    // just yields an auth error below, and reauth re-spawns this task.)
    let creds = match sync::imap_creds_for(state, account_id).await {
        Ok(c) => c,
        Err(_) => return ConnEnd::Stop,
    };
    let mut session = match state.net.imap.open(creds).await {
        Ok(s) => s,
        Err(e) if sync::is_auth_error(&e) => {
            let _ = AccountRepo::new(state.storage.db())
                .set_auth_failed(account_id)
                .await;
            return ConnEnd::Stop;
        }
        Err(_) => return ConnEnd::Reconnect,
    };

    // IDLE requires a selected mailbox.
    if let Err(e) = session.select_inbox().await {
        return if sync::is_auth_error(&e) {
            ConnEnd::Stop
        } else {
            ConnEnd::Reconnect
        };
    }
    // The connection is healthy — clear the reconnect backoff so a later transient
    // drop retries quickly (1s) rather than at the capped interval.
    *consecutive_errors = 0;

    let keepalive = Duration::from_secs(IDLE_KEEPALIVE_SECS);
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return ConnEnd::Stop,
            res = session.idle_wait(keepalive) => match res {
                // Any server change → hand off to the poll task, which owns all
                // fetching. `notify_one` stores a permit if the poll task is busy,
                // so the wake is never lost.
                Ok(IdleOutcome::MailArrived) => notify.notify_one(),
                // Keepalive elapsed → re-enter IDLE on the next loop turn.
                Ok(IdleOutcome::TimedOut) => {}
                Err(e) if sync::is_auth_error(&e) => {
                    let _ = AccountRepo::new(state.storage.db())
                        .set_auth_failed(account_id)
                        .await;
                    return ConnEnd::Stop;
                }
                Err(_) => return ConnEnd::Reconnect,
            },
        }
    }
}

/// Sleep for `dur`, or return early if cancelled. Returns `true` when cancelled.
async fn sleep_or_cancelled(cancel: &CancellationToken, dur: Duration) -> bool {
    tokio::select! {
        _ = cancel.cancelled() => true,
        _ = tokio::time::sleep(dur) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::fakes::{net_with_imap, FakeImapFactory, FakeMailbox};
    use crate::state::AppState;

    /// A scripted `MailArrived` must poke the poll `Notify`, proving the listener
    /// hands fetching off to the poll task instead of fetching itself.
    #[tokio::test]
    async fn mail_arrived_pokes_the_poll_notify() {
        let account_id = "5f2d6a1e-0000-4000-8000-0000000000ab";
        let mailbox = FakeMailbox::new()
            .with_inbox(1, 1, 0)
            .with_idle_outcomes([IdleOutcome::MailArrived]);
        let (state, _rx) =
            AppState::test_state_with_net(net_with_imap(FakeImapFactory::new(mailbox))).await;

        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, imap_host, color_token, \
                 badge_label, created_at, updated_at) \
             VALUES (?, 'a@x.com', 'A', 'imap', 'imap.example.com', 'slate', 'A', 0, 0)",
        )
        .bind(account_id)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        SyncStateRepo::new(state.storage.db())
            .ensure(account_id)
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        let notify = Arc::new(Notify::new());
        let handle = spawn(
            state,
            account_id.to_string(),
            cancel.clone(),
            notify.clone(),
        );

        // The first idle_wait yields MailArrived → notify fires; the second finds an
        // empty script and parks on the keepalive sleep. So awaiting `notified()`
        // must resolve promptly.
        tokio::time::timeout(Duration::from_secs(5), notify.notified())
            .await
            .expect("idle listener should poke the poll notify on MailArrived");

        cancel.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
    }

    /// A deleted / unconfigured account makes credential lookup fail; the listener
    /// must exit cleanly rather than spin.
    #[tokio::test]
    async fn unknown_account_stops_cleanly() {
        let (state, _rx) = AppState::test_state().await;
        let cancel = CancellationToken::new();
        let notify = Arc::new(Notify::new());
        let handle = spawn(state, "no-such-account".into(), cancel, notify);
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("listener should exit promptly for an unknown account")
            .unwrap();
    }
}
