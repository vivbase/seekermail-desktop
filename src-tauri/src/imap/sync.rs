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

    // 5. SEARCH new UIDs, then drop any already on disk. An IMAP `UID n:*` range
    //    always echoes the mailbox's highest UID even when nothing is newer than
    //    the cursor (RFC 3501 §6.4.8), so the result can contain UIDs we already
    //    hold. Keep the server's true high-water mark for the cursor (step 7), but
    //    fetch bodies only for UIDs not yet persisted (T022 §3, F_A4 §4.6) — so a
    //    poll with no new mail re-downloads nothing.
    let found = session.search_uids_from(uid_next).await?;
    let server_max_uid = found.iter().copied().max();
    let mut uids = Vec::with_capacity(found.len());
    for uid in found {
        if !super::dedup::is_duplicate(state.storage.db(), account_id, "INBOX", uid).await? {
            uids.push(uid);
        }
    }
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

    // 7. Advance the cursor to the server's high-water mark — computed from the
    //    raw SEARCH result, so de-duplication can never make the cursor regress.
    ss_repo
        .update_after_poll(
            account_id,
            SyncOutcome {
                inbox_uid_validity: Some(status.uid_validity),
                inbox_uid_next: server_max_uid.map(|m| m + 1).or(Some(uid_next)),
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

    /// A poll must skip UIDs already on disk (the dedup seam, T022 §3): because an
    /// IMAP `UID n:*` SEARCH echoes the highest UID even when nothing is new, the
    /// boundary message would otherwise be re-fetched on every poll. The cursor
    /// must still advance to the server's high-water mark, not the filtered max.
    #[tokio::test]
    async fn poll_skips_uids_already_on_disk() {
        use crate::net::fakes::{net_with_imap, FakeImapFactory, FakeMailbox};
        use crate::storage::MailRepo;
        use crate::types::ParsedMail;

        let account_id = "5f2d6a1e-0000-4000-8000-0000000000aa";
        // Mailbox holds UID 5 (already synced locally) and UID 6 (genuinely new).
        let mailbox = FakeMailbox::new()
            .with_inbox(1, 7, 2)
            .with_uids([5, 6])
            .with_body(5, b"raw-5".to_vec())
            .with_body(6, b"raw-6".to_vec());
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

        // Cursor sits at UID 5, so the SEARCH echoes the already-synced boundary.
        let ss = SyncStateRepo::new(state.storage.db());
        ss.ensure(account_id).await.unwrap();
        ss.update_after_poll(
            account_id,
            SyncOutcome {
                inbox_uid_validity: Some(1),
                inbox_uid_next: Some(5),
                new_mails: 0,
            },
        )
        .await
        .unwrap();

        // UID 5 is already persisted.
        MailRepo::new(state.storage.db())
            .upsert_batch(&[ParsedMail {
                account_id: account_id.into(),
                folder: "INBOX".into(),
                imap_uid: Some(5),
                message_id: "<5@x>".into(),
                in_reply_to: None,
                references: None,
                subject: "S".into(),
                from_name: None,
                from_email: "a@x.com".into(),
                to_addrs: "[]".into(),
                cc_addrs: "[]".into(),
                bcc_addrs: "[]".into(),
                reply_to: None,
                date_sent: 1,
                date_received: 1,
                body_text: Some("b".into()),
                body_html: None,
                snippet: Some("b".into()),
                has_attachments: false,
                tracker_count: 0,
                attachments: vec![],
            }])
            .await
            .unwrap();

        // Only UID 6 should be queued — UID 5 is filtered out (would be 2 without dedup).
        let queued = poll_once(&state, account_id).await.unwrap();
        assert_eq!(queued, 1, "already-synced UID 5 must be skipped");

        // Cursor advances to the server high-water mark (max(5,6) + 1), not the
        // filtered max — proving dedup cannot make the cursor regress.
        assert_eq!(ss.get(account_id).await.unwrap().inbox_uid_next, Some(7));
    }
}
