//! Outbound write-back drain (Phase 2 two-way sync). Applies queued
//! [`outbound_ops`](crate::storage::outbound_op_repo) to the IMAP server via
//! `UID STORE`. Triggered right after a local action (for promptness) via
//! [`spawn_drain`], and again from the poll loop as a safety net. Best-effort: a
//! connection failure leaves the ops pending for the next drain; a per-op failure
//! retries up to the queue's attempt cap.

use crate::error::AppResult;
use crate::net::{FolderRole, ImapFlag};
use crate::state::AppState;
use crate::storage::{OutboundOpKind, OutboundOpRepo};

/// Max ops applied per drain pass — keeps one connection's work bounded.
const DRAIN_BATCH: i64 = 100;

/// The `(flag, set)` a flag op performs, or `None` for a move op.
fn op_flag(kind: OutboundOpKind) -> Option<(ImapFlag, bool)> {
    match kind {
        OutboundOpKind::MarkSeen => Some((ImapFlag::Seen, true)),
        OutboundOpKind::MarkUnseen => Some((ImapFlag::Seen, false)),
        OutboundOpKind::Flag => Some((ImapFlag::Flagged, true)),
        OutboundOpKind::Unflag => Some((ImapFlag::Flagged, false)),
        _ => None,
    }
}

/// Apply this account's pending write-backs. Opens one IMAP session, resolves the
/// server's folder names once (SPECIAL-USE, since the local `folder` tag is not
/// the server mailbox name), then for each op applies a flag `STORE` or a `MOVE`
/// and marks it done or (on failure) schedules a retry. Returns the number applied.
pub async fn drain(state: &AppState, account_id: &str) -> AppResult<u32> {
    let repo = OutboundOpRepo::new(state.storage.db());
    let ops = repo.claim_pending(account_id, DRAIN_BATCH).await?;
    if ops.is_empty() {
        return Ok(0);
    }

    // OAuth pre-flight (no-op for IMAP password accounts), mirroring the poll path.
    if crate::account::refresh::needs_refresh(state, account_id).await? {
        crate::account::refresh::refresh_oauth(state, account_id).await?;
    }
    let creds = super::sync::imap_creds_for(state, account_id).await?;
    let mut session = state.net.imap.open(creds).await?;

    // Resolve folder roles → live server mailbox names once for this drain. INBOX
    // is always "INBOX"; everything else may be provider-specific (`[Gmail]/…`).
    let folders = session.list_folders().await?;
    let server_for = |role: FolderRole| -> Option<String> {
        if role == FolderRole::Inbox {
            return Some("INBOX".to_string());
        }
        folders
            .iter()
            .find(|f| f.role == role)
            .map(|f| f.name.clone())
    };

    let mut applied = 0u32;
    for op in ops {
        // The message's current server mailbox, mapped from its stored tag.
        let Some(source) = FolderRole::from_local_tag(&op.folder).and_then(server_for) else {
            // Unknown / unsupported source folder → nothing to sync; the local
            // action already stands. Mark done so it doesn't retry forever.
            tracing::debug!(account_id = %account_id, folder = %op.folder, "write-back: unresolved source folder, skipping");
            repo.mark_done(&op.id).await?;
            continue;
        };

        let result = if let Some((flag, set)) = op_flag(op.kind) {
            session.store_flag(&source, op.imap_uid, flag, set).await
        } else {
            // Move op → resolve the destination folder by role.
            let dest = match op.kind {
                OutboundOpKind::Trash => server_for(FolderRole::Trash),
                OutboundOpKind::MarkSpam => server_for(FolderRole::Junk),
                // Archive = a real Archive folder, else All Mail (Gmail's archive
                // is "remove from INBOX", which All Mail represents).
                OutboundOpKind::Archive => {
                    server_for(FolderRole::Archive).or_else(|| server_for(FolderRole::All))
                }
                // Restore = move back to the INBOX. When the source is already INBOX
                // (an undo that coalesced away a pending Trash move before it drained),
                // dest == source below makes this a no-op.
                OutboundOpKind::Restore => server_for(FolderRole::Inbox),
                _ => None,
            };
            match dest {
                Some(dest) if dest != source => {
                    session.move_message(&source, op.imap_uid, &dest).await
                }
                // No destination folder, or already there → nothing to move.
                _ => {
                    tracing::debug!(account_id = %account_id, op = op.kind.as_str(), "write-back: no destination folder, skipping move");
                    Ok(())
                }
            }
        };

        match result {
            Ok(()) => {
                repo.mark_done(&op.id).await?;
                applied += 1;
            }
            Err(e) => {
                repo.mark_failed(&op.id, op.attempts, &format!("{e}"))
                    .await?;
            }
        }
    }
    Ok(applied)
}

/// Fire-and-forget drain for one account, run right after a local action so the
/// change reaches the server promptly without blocking the command/UI.
pub fn spawn_drain(state: AppState, account_id: String) {
    tokio::spawn(async move {
        if let Err(e) = drain(&state, &account_id).await {
            tracing::warn!(account_id = %account_id, error = %e, "outbound drain failed");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::fakes::{net_with_imap, FakeImapFactory, FakeMailbox};

    #[tokio::test]
    async fn drain_applies_store_flags_and_marks_done() {
        let factory = FakeImapFactory::new(FakeMailbox::new());
        let log = factory.log();
        let (state, _rx) = AppState::test_state_with_net(net_with_imap(factory)).await;

        let account_id = "5f2d6a1e-0000-4000-8000-0000000000e1";
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, imap_host, color_token, \
                 badge_label, created_at, updated_at) \
             VALUES (?, 'a@x.com', 'A', 'imap', 'imap.example.com', 'slate', 'A', 0, 0)",
        )
        .bind(account_id)
        .execute(state.storage.db().pool())
        .await
        .unwrap();

        let repo = OutboundOpRepo::new(state.storage.db());
        repo.enqueue(account_id, "INBOX", 5, OutboundOpKind::MarkSeen)
            .await
            .unwrap();
        repo.enqueue(account_id, "INBOX", 6, OutboundOpKind::Flag)
            .await
            .unwrap();

        let applied = drain(&state, account_id).await.unwrap();
        assert_eq!(applied, 2);

        // The fake recorded both UID STOREs with the right flag + folder.
        let calls = log.lock().unwrap().clone();
        assert!(calls.iter().any(|c| c == "store_flag:INBOX:5:\\Seen:true"));
        assert!(calls
            .iter()
            .any(|c| c == "store_flag:INBOX:6:\\Flagged:true"));

        // Nothing left pending — both ops are done.
        assert!(repo.claim_pending(account_id, 50).await.unwrap().is_empty());
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

    /// Move ops resolve the destination by SPECIAL-USE role and issue `UID MOVE`
    /// from the source to the live (provider-specific) server folder name.
    #[tokio::test]
    async fn drain_moves_to_resolved_trash_and_junk_folders() {
        let mailbox = FakeMailbox::new().with_folders([
            ("INBOX", FolderRole::Inbox),
            ("[Gmail]/Trash", FolderRole::Trash),
            ("[Gmail]/Spam", FolderRole::Junk),
        ]);
        let factory = FakeImapFactory::new(mailbox);
        let log = factory.log();
        let (state, _rx) = AppState::test_state_with_net(net_with_imap(factory)).await;

        let account_id = "5f2d6a1e-0000-4000-8000-0000000000e2";
        account(&state, account_id).await;

        let repo = OutboundOpRepo::new(state.storage.db());
        repo.enqueue(account_id, "INBOX", 11, OutboundOpKind::Trash)
            .await
            .unwrap();
        repo.enqueue(account_id, "INBOX", 12, OutboundOpKind::MarkSpam)
            .await
            .unwrap();

        let applied = drain(&state, account_id).await.unwrap();
        assert_eq!(applied, 2);

        let calls = log.lock().unwrap().clone();
        assert!(calls.iter().any(|c| c == "move:INBOX:11:[Gmail]/Trash"));
        assert!(calls.iter().any(|c| c == "move:INBOX:12:[Gmail]/Spam"));
    }

    /// When the destination role has no folder on the server, the move is a no-op
    /// and the op is cleared (not stuck retrying) — the local action still stands.
    #[tokio::test]
    async fn drain_skips_move_when_destination_folder_missing() {
        // Only INBOX exists — no Trash folder to move into.
        let mailbox = FakeMailbox::new().with_folders([("INBOX", FolderRole::Inbox)]);
        let factory = FakeImapFactory::new(mailbox);
        let log = factory.log();
        let (state, _rx) = AppState::test_state_with_net(net_with_imap(factory)).await;

        let account_id = "5f2d6a1e-0000-4000-8000-0000000000e3";
        account(&state, account_id).await;

        let repo = OutboundOpRepo::new(state.storage.db());
        repo.enqueue(account_id, "INBOX", 5, OutboundOpKind::Trash)
            .await
            .unwrap();

        let applied = drain(&state, account_id).await.unwrap();
        assert_eq!(applied, 1, "no-op move still clears the op");
        assert!(
            !log.lock().unwrap().iter().any(|c| c.starts_with("move:")),
            "no UID MOVE issued without a destination"
        );
        assert!(repo.claim_pending(account_id, 50).await.unwrap().is_empty());
    }

    /// A restore op moves the message from its (Trash) source back to the INBOX.
    #[tokio::test]
    async fn drain_restore_moves_back_to_inbox() {
        let mailbox = FakeMailbox::new().with_folders([
            ("INBOX", FolderRole::Inbox),
            ("[Gmail]/Trash", FolderRole::Trash),
        ]);
        let factory = FakeImapFactory::new(mailbox);
        let log = factory.log();
        let (state, _rx) = AppState::test_state_with_net(net_with_imap(factory)).await;

        let account_id = "5f2d6a1e-0000-4000-8000-0000000000e4";
        account(&state, account_id).await;

        let repo = OutboundOpRepo::new(state.storage.db());
        // The local row's tag is TRASH (move-detection followed it there); restore
        // moves it back to the INBOX.
        repo.enqueue(account_id, "TRASH", 30, OutboundOpKind::Restore)
            .await
            .unwrap();

        let applied = drain(&state, account_id).await.unwrap();
        assert_eq!(applied, 1);
        assert!(log
            .lock()
            .unwrap()
            .iter()
            .any(|c| c == "move:[Gmail]/Trash:30:INBOX"));
    }
}
