//! Inbound reconciliation (Phase 2 two-way sync, server→local). Re-reads the
//! server's FLAGS for the most recent window of locally-held INBOX messages,
//! mirrors read/star state into the local DB, and marks messages that have
//! vanished from the server folder (moved or deleted elsewhere) as archived
//! locally. Runs from the poll loop as a safety-net pass.
//!
//! Best-effort and idempotent. It skips any message that still has a pending
//! outbound op, so reading the server's (older) state can never clobber a local
//! change that hasn't been written back yet.

use std::collections::{HashMap, HashSet};

use crate::config::RECONCILE_WINDOW;
use crate::error::AppResult;
use crate::net::MessageFlags;
use crate::state::AppState;
use crate::storage::{MailRepo, OutboundOpRepo};

/// Reconcile INBOX read/star state and vanished messages for one account. Returns
/// the number of local rows changed.
pub async fn reconcile(state: &AppState, account_id: &str) -> AppResult<u32> {
    const FOLDER: &str = "INBOX";
    let mail = MailRepo::new(state.storage.db());

    // Window: the most recent RECONCILE_WINDOW locally-held UIDs.
    let Some(local_max) = mail.local_max_uid(account_id, FOLDER).await? else {
        return Ok(0); // nothing local yet
    };
    let window_start = (local_max - RECONCILE_WINDOW).max(1);
    let local = mail
        .local_flag_window(account_id, FOLDER, window_start)
        .await?;
    if local.is_empty() {
        return Ok(0);
    }

    // Messages with a pending outbound op are authoritative locally until synced.
    let pending: HashSet<(String, i64)> = OutboundOpRepo::new(state.storage.db())
        .pending_targets(account_id)
        .await?
        .into_iter()
        .collect();

    // OAuth pre-flight, then read the server's current flags for the window.
    if crate::account::refresh::needs_refresh(state, account_id).await? {
        crate::account::refresh::refresh_oauth(state, account_id).await?;
    }
    let creds = super::sync::imap_creds_for(state, account_id).await?;
    let mut session = state.net.imap.open(creds).await?;
    let server: HashMap<i64, MessageFlags> = session
        .fetch_flags(FOLDER, window_start)
        .await?
        .into_iter()
        .collect();

    let mut changed = 0u32;
    for (uid, l_read, l_starred) in local {
        if pending.contains(&(FOLDER.to_string(), uid)) {
            continue;
        }
        match server.get(&uid) {
            Some(flags) => {
                if flags.seen != l_read || flags.flagged != l_starred {
                    mail.set_flags_by_uid(account_id, FOLDER, uid, flags.seen, flags.flagged)
                        .await?;
                    changed += 1;
                }
            }
            None => {
                // Vanished from the server INBOX → reflect locally as archived
                // (removed from the active stream; non-destructive).
                mail.set_archived_by_uid(account_id, FOLDER, uid).await?;
                changed += 1;
            }
        }
    }

    if changed > 0 {
        // Nudge the UI to re-query the now-updated streams.
        state.events.sync_complete(account_id, 0);
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::fakes::{net_with_imap, FakeImapFactory, FakeMailbox};
    use crate::storage::OutboundOpKind;
    use crate::types::ParsedMail;

    fn pm(account: &str, uid: i64, msgid: &str) -> ParsedMail {
        ParsedMail {
            account_id: account.into(),
            folder: "INBOX".into(),
            imap_uid: Some(uid),
            message_id: msgid.into(),
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
        }
    }

    async fn account(state: &AppState, id: &str) {
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

    async fn flags_row(state: &AppState, account_id: &str, uid: i64) -> (i64, i64, i64) {
        sqlx::query_as(
            "SELECT is_read, is_starred, is_archived FROM mails \
             WHERE account_id = ? AND folder = 'INBOX' AND imap_uid = ?",
        )
        .bind(account_id)
        .bind(uid)
        .fetch_one(state.storage.db().pool())
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn reconcile_mirrors_server_flags_and_archives_vanished() {
        // Server: 10 read, 11 starred, 12 absent (vanished). Local: all unread.
        let mailbox = FakeMailbox::new()
            .with_message_flags("INBOX", 10, true, false)
            .with_message_flags("INBOX", 11, false, true);
        let (state, _rx) =
            AppState::test_state_with_net(net_with_imap(FakeImapFactory::new(mailbox))).await;

        let account_id = "5f2d6a1e-0000-4000-8000-0000000000f1";
        account(&state, account_id).await;
        MailRepo::new(state.storage.db())
            .upsert_batch(&[
                pm(account_id, 10, "<10@x>"),
                pm(account_id, 11, "<11@x>"),
                pm(account_id, 12, "<12@x>"),
            ])
            .await
            .unwrap();

        let changed = reconcile(&state, account_id).await.unwrap();
        assert_eq!(changed, 3);

        assert_eq!(flags_row(&state, account_id, 10).await.0, 1, "10 → read");
        assert_eq!(flags_row(&state, account_id, 11).await.1, 1, "11 → starred");
        assert_eq!(
            flags_row(&state, account_id, 12).await.2,
            1,
            "12 vanished → archived"
        );
    }

    #[tokio::test]
    async fn reconcile_skips_messages_with_pending_outbound_op() {
        // User just read uid 20 locally (is_read=1) and it's queued for write-back;
        // the server still reports it unseen. Reconcile must NOT revert it.
        let mailbox = FakeMailbox::new().with_message_flags("INBOX", 20, false, false);
        let (state, _rx) =
            AppState::test_state_with_net(net_with_imap(FakeImapFactory::new(mailbox))).await;

        let account_id = "5f2d6a1e-0000-4000-8000-0000000000f2";
        account(&state, account_id).await;
        let mail = MailRepo::new(state.storage.db());
        mail.upsert_batch(&[pm(account_id, 20, "<20@x>")])
            .await
            .unwrap();
        mail.set_flags_by_uid(account_id, "INBOX", 20, true, false)
            .await
            .unwrap();
        OutboundOpRepo::new(state.storage.db())
            .enqueue(account_id, "INBOX", 20, OutboundOpKind::MarkSeen)
            .await
            .unwrap();

        let changed = reconcile(&state, account_id).await.unwrap();
        assert_eq!(changed, 0, "pending op shields the local change");
        assert_eq!(
            flags_row(&state, account_id, 20).await.0,
            1,
            "local read state preserved"
        );
    }
}
