//! Incremental IMAP sync (`poll_once`) over the transport seam (T021 + T022).
//!
//! One poll: optional OAuth pre-refresh → connect → `SELECT INBOX` → UIDVALIDITY
//! check → `SEARCH UID uid_next:*` → `FETCH` new bodies in batches onto the ingest
//! channel → advance the `sync_state` cursor. Error classification (auth vs
//! transient) is exposed via [`is_auth_error`] for the poll task to act on.

use crate::config::POLL_FETCH_BATCH_SIZE;
use crate::error::{AppError, AppResult};
use crate::keychain::CredKind;
use crate::net::ImapCreds;
use crate::state::AppState;
use crate::storage::sync_state_repo::SyncOutcome;
use crate::storage::{AccountRepo, SyncStateRepo};
use crate::types::{ErrorCode, RawMail};
use crate::util::{now_unix, parse_uuid};

/// Run one incremental poll. Returns the number of new bodies queued for parsing.
pub async fn poll_once(state: &AppState, account_id: &str) -> AppResult<u32> {
    // 1. OAuth pre-flight refresh (no-op for IMAP password accounts).
    if crate::account::refresh::needs_refresh(state, account_id).await? {
        crate::account::refresh::refresh_oauth(state, account_id).await?;
    }

    // 2. Connect.
    let creds = imap_creds_for(state, account_id).await?;
    let mut session = state.net.imap.open(creds).await?;

    // 3. SELECT INBOX.
    let status = session.select_inbox().await?;
    let ss_repo = SyncStateRepo::new(state.storage.db());
    let prev = ss_repo.get(account_id).await.ok();
    let prev_validity = prev.as_ref().and_then(|s| s.inbox_uid_validity);
    let uid_next = prev.as_ref().and_then(|s| s.inbox_uid_next).unwrap_or(1);

    // 4. UIDVALIDITY change → mark for full resync, let backfill handle it.
    if let Some(prev_v) = prev_validity {
        if prev_v != status.uid_validity {
            ss_repo
                .flag_uid_validity_change(account_id, status.uid_validity)
                .await?;
            state.events.sync_error(
                account_id,
                ErrorCode::ImapUidValidityChanged,
                "mailbox needs a full resync",
            );
            return Ok(0);
        }
    }

    // 5. SEARCH new UIDs.
    let uids = session.search_uids_from(uid_next).await?;
    state.events.sync_started(account_id);
    let total = uids.len() as u32;

    // 6. FETCH bodies in batches onto the ingest channel.
    let mut sent = 0u32;
    for chunk in uids.chunks(POLL_FETCH_BATCH_SIZE) {
        let bodies = session.fetch_bodies(chunk).await?;
        for (uid, bytes) in bodies {
            // Back-pressure: await capacity so fetch can't outrun parsing.
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
                // Receiver gone → app shutting down.
                return Ok(sent);
            }
            sent += 1;
        }
        state
            .events
            .sync_progress(account_id, sent, Some(total), false);
    }

    // 7. Advance the cursor.
    let max_uid = uids.iter().copied().max();
    ss_repo
        .update_after_poll(
            account_id,
            SyncOutcome {
                inbox_uid_validity: Some(status.uid_validity),
                inbox_uid_next: max_uid.map(|m| m + 1).or(Some(uid_next)),
                new_mails: sent,
            },
        )
        .await?;
    AccountRepo::new(state.storage.db())
        .set_last_synced(account_id, now_unix())
        .await?;
    state.events.sync_complete(account_id, sent);
    Ok(sent)
}

/// Build IMAP credentials for an account (shared with the attachment downloader).
pub(crate) async fn imap_creds_for(state: &AppState, account_id: &str) -> AppResult<ImapCreds> {
    let acct = AccountRepo::new(state.storage.db()).get(account_id).await?;
    let uuid = parse_uuid(account_id)?;
    let secret = state
        .keychain
        .get(&uuid, CredKind::ImapPassword)?
        .or(state.keychain.get(&uuid, CredKind::OAuthAccessToken)?)
        .map(|s| s.expose().to_string())
        .unwrap_or_default();
    let host = acct
        .imap_host
        .ok_or_else(|| AppError::ImapConnection("no imap host configured".into()))?;
    Ok(ImapCreds {
        host,
        port: acct.imap_port,
        tls: acct.imap_port == 993,
        email: acct.email,
        secret,
    })
}

/// Auth-class errors stop the account's poll loop; everything else backs off.
pub fn is_auth_error(err: &AppError) -> bool {
    matches!(
        err,
        AppError::AuthInvalidCredentials
            | AppError::AuthOAuthFailed(_)
            | AppError::AuthKeychainDenied
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_classification() {
        assert!(is_auth_error(&AppError::AuthInvalidCredentials));
        assert!(is_auth_error(&AppError::AuthOAuthFailed("x".into())));
        assert!(!is_auth_error(&AppError::ImapConnection("net".into())));
        assert!(!is_auth_error(&AppError::FsDiskFull));
    }
}
