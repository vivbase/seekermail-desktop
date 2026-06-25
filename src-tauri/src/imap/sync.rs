//! Incremental IMAP sync (`poll_once`) over the transport seam (T021 + T022).
//!
//! One poll: optional OAuth pre-refresh → connect → `SELECT INBOX` → UIDVALIDITY
//! check → `SEARCH UID uid_next:*` → `FETCH` new bodies in batches onto the ingest
//! channel → advance the `sync_state` cursor. Error classification (auth vs
//! transient) is exposed via [`is_auth_error`] for the poll task to act on.

use crate::config::{POLL_FETCH_BATCH_SIZE, SECONDARY_INITIAL_WINDOW};
use crate::error::{AppError, AppResult};
use crate::keychain::CredKind;
use crate::net::ImapCreds;
use crate::state::AppState;
use crate::storage::sync_state_repo::SyncOutcome;
use crate::storage::{AccountRepo, FolderSyncOutcome, FolderSyncStateRepo, SyncStateRepo};
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
    let prev_uid_next = prev.as_ref().and_then(|s| s.inbox_uid_next);

    // 3a. First sync of a brand-new account → establish the incremental baseline at
    //     the mailbox's *current* high-water mark and fetch nothing here. The poll
    //     then only ever handles mail that arrives from now on; everything already
    //     on the server is the history backfill's job (T022), and the two never
    //     overlap on the UID axis (F_A4 §6: "the incremental poll's start point is
    //     set to the current latest UID at account-add time"). A `sync_state` row
    //     with neither a cursor nor a recorded UIDVALIDITY is the unambiguous
    //     "never synced" state — the create-time seed leaves both NULL. A
    //     UIDVALIDITY reset, by contrast, clears only the cursor and keeps a
    //     validity (see `flag_uid_validity_change`), so it still falls through to
    //     the full re-scan below. Without this baseline the cursor would default to
    //     1 and every "first" poll would re-scan the whole mailbox (`UID 1:*`,
    //     violating §7.3); on a large mailbox that never finishes inside the
    //     per-poll budget, so the cursor never advances and genuinely new mail (the
    //     highest UIDs, fetched last) is never reached.
    if prev_uid_next.is_none() && prev_validity.is_none() {
        ss_repo
            .update_after_poll(
                account_id,
                SyncOutcome {
                    inbox_uid_validity: Some(status.uid_validity),
                    inbox_uid_next: Some(status.uid_next.max(1)),
                    new_mails: 0,
                },
            )
            .await?;
        AccountRepo::new(state.storage.db())
            .set_last_synced(account_id, now_unix())
            .await?;
        // Existing history is the backfill's job (T022) — but it has to actually be
        // started, or a freshly added mailbox imports nothing (the incremental poll
        // only ever fetches mail that arrives after this baseline). Kicking it off
        // here makes import reliable for every add path — wizard, onboarding,
        // re-enable, programmatic — not just the one that calls set_knowledge_depth.
        // The per-account guard in `backfill` collapses this with the wizard's own
        // trigger into a single run, and an unset knowledge depth backfills all.
        super::backfill::spawn_start(state.clone(), account_id.to_string());
        state.events.sync_complete(account_id, 0);
        return Ok(0);
    }

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
    let uid_next = prev_uid_next.unwrap_or(1);
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
    // New mail may have created or grown threads — refresh a few thread
    // summaries off the critical path so the agent's "memory" stays current
    // (analysis/54 §3.5, P-4). Best-effort, provider-gated; never blocks sync.
    if sent > 0 {
        crate::ai::memory::spawn_summary_build(state, account_id);
    }
    Ok(sent)
}

/// Sync the secondary folders (SENT / JUNK / TRASH) for one account — the
/// complement to [`poll_once`], which owns the INBOX. Discovers folders via
/// SPECIAL-USE (`list_folders`), then for each allow-listed non-INBOX folder runs
/// an incremental UID sync against its own cursor ([`FolderSyncStateRepo`]). A
/// folder's first sync pulls a bounded recent window ([`SECONDARY_INITIAL_WINDOW`])
/// so recent mail shows up immediately without a full history backfill. Returns the
/// number of bodies queued for parsing.
pub async fn poll_secondary_folders(state: &AppState, account_id: &str) -> AppResult<u32> {
    // OAuth pre-flight (no-op for IMAP password accounts), mirroring `poll_once`.
    if crate::account::refresh::needs_refresh(state, account_id).await? {
        crate::account::refresh::refresh_oauth(state, account_id).await?;
    }
    let creds = imap_creds_for(state, account_id).await?;
    let mut session = state.net.imap.open(creds).await?;

    let folders = session.list_folders().await?;
    let repo = FolderSyncStateRepo::new(state.storage.db());
    let mut total = 0u32;

    for folder in folders {
        // Read-side allow-list: SENT / JUNK / TRASH. INBOX is `poll_once`'s job;
        // Drafts/Archive/All/Other are not ingested in this pass (`local_folder_tag`).
        let tag = match folder.role.local_folder_tag() {
            Some(t) if t != "INBOX" => t,
            _ => continue,
        };
        total += sync_secondary_folder(
            state,
            &repo,
            session.as_mut(),
            account_id,
            &folder.name,
            tag,
        )
        .await?;
    }
    Ok(total)
}

/// One secondary folder's incremental pass. Split out so `session` is reborrowed
/// per folder. Tags every queued body with `tag` (SENT/JUNK/TRASH) so the parse
/// worker applies the right ingest policy.
async fn sync_secondary_folder(
    state: &AppState,
    repo: &FolderSyncStateRepo<'_>,
    session: &mut dyn crate::net::ImapSession,
    account_id: &str,
    folder_name: &str,
    tag: &str,
) -> AppResult<u32> {
    repo.ensure(account_id, tag).await?;
    let status = session.select_folder(folder_name).await?;
    let prev = repo.get_opt(account_id, tag).await?;
    let prev_validity = prev.as_ref().and_then(|s| s.uid_validity);
    let prev_uid_next = prev.as_ref().and_then(|s| s.uid_next);

    // A UIDVALIDITY change invalidates the saved cursor (RFC 3501 §2.3.1.1).
    let validity_changed = matches!(prev_validity, Some(pv) if pv != status.uid_validity);
    if validity_changed {
        repo.flag_uid_validity_change(account_id, tag, status.uid_validity)
            .await?;
    }
    // First sync (or after a validity reset) → start from a bounded recent window;
    // otherwise resume from the saved high-water mark.
    let from = if prev_uid_next.is_none() || validity_changed {
        (status.uid_next - SECONDARY_INITIAL_WINDOW).max(1)
    } else {
        prev_uid_next.unwrap_or(1)
    };

    let found = session.search_uids_from(from).await?;
    let server_max = found.iter().copied().max();
    let mut uids = Vec::with_capacity(found.len());
    for uid in found {
        if !super::dedup::is_duplicate(state.storage.db(), account_id, tag, uid).await? {
            uids.push(uid);
        }
    }

    let mut sent = 0u32;
    for chunk in uids.chunks(POLL_FETCH_BATCH_SIZE) {
        let bodies = session.fetch_bodies(chunk).await?;
        for (uid, bytes) in bodies {
            if state
                .mail_tx
                .send(RawMail {
                    account_id: account_id.to_string(),
                    folder: tag.to_string(),
                    imap_uid: uid,
                    raw_bytes: bytes,
                })
                .await
                .is_err()
            {
                return Ok(sent); // receiver gone → shutting down
            }
            sent += 1;
        }
    }

    repo.update_after_poll(
        account_id,
        tag,
        FolderSyncOutcome {
            uid_validity: Some(status.uid_validity),
            uid_next: server_max.map(|m| m + 1).or(Some(from)),
            new_mails: sent,
        },
    )
    .await?;
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

    /// A brand-new account's very first poll must NOT re-scan the whole mailbox.
    /// Per F_A4 §6 the incremental cursor is seeded to the mailbox's current
    /// high-water mark and existing history is left to the backfill — so the poll
    /// itself queues nothing and just records the baseline (`inbox_uid_next` =
    /// mailbox UIDNEXT, `inbox_uid_validity` = mailbox UIDVALIDITY). Without the
    /// baseline the cursor would default to 1 and the poll would pull every
    /// existing message (here UIDs 95/96/97) on every "first" poll. (The first
    /// poll also *starts* that backfill so history is actually imported — the
    /// delivery side is covered by `first_sync_imports_existing_mail_for_new_account`;
    /// here `_rx` is dropped, so the spawned backfill's sends simply no-op.)
    #[tokio::test]
    async fn first_poll_seeds_baseline_without_refetching_history() {
        use crate::net::fakes::{net_with_imap, FakeImapFactory, FakeMailbox};

        let account_id = "5f2d6a1e-0000-4000-8000-0000000000ac";
        // Non-empty mailbox: UIDVALIDITY 42, UIDNEXT 100, three existing messages
        // all below the high-water mark (history the backfill owns, not the poll).
        let mailbox = FakeMailbox::new()
            .with_inbox(42, 100, 3)
            .with_uids([95, 96, 97])
            .with_body(95, b"raw-95".to_vec())
            .with_body(96, b"raw-96".to_vec())
            .with_body(97, b"raw-97".to_vec());
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

        // Fresh sync_state: NULL cursor AND NULL validity — the "never synced" state.
        let ss = SyncStateRepo::new(state.storage.db());
        ss.ensure(account_id).await.unwrap();

        // First poll establishes the baseline and queues nothing (would be 3 if it
        // re-scanned from UID 1).
        let queued = poll_once(&state, account_id).await.unwrap();
        assert_eq!(queued, 0, "first poll must not pull existing history");

        // Cursor seeded to the mailbox high-water mark; validity recorded.
        let s = ss.get(account_id).await.unwrap();
        assert_eq!(s.inbox_uid_next, Some(100));
        assert_eq!(s.inbox_uid_validity, Some(42));

        // A second poll now searches forward only (UID 100:*) and still finds
        // nothing — the three history messages stay the backfill's responsibility.
        let queued2 = poll_once(&state, account_id).await.unwrap();
        assert_eq!(queued2, 0);
        assert_eq!(ss.get(account_id).await.unwrap().inbox_uid_next, Some(100));
    }

    /// Regression for the 2026-06-21 import break ("mailbox added → no mail shows").
    /// A brand-new account's first sync establishes the UID baseline AND must kick
    /// off the history backfill, so the mail already on the server is imported —
    /// without depending on the user reaching the knowledge-depth step (until now
    /// the only place a backfill was ever started). The poll itself still queues
    /// nothing; the existing messages arrive on the ingest channel via the backfill
    /// the first sync starts.
    #[tokio::test]
    async fn first_sync_imports_existing_mail_for_new_account() {
        use crate::net::fakes::{net_with_imap, FakeImapFactory, FakeMailbox};

        let account_id = "5f2d6a1e-0000-4000-8000-0000000000c1";
        let body = |uid: i64| -> Vec<u8> {
            format!(
                "From: sender{uid}@example.com\r\nSubject: Message {uid}\r\n\
                 Message-ID: <{uid}@example.com>\r\n\r\nBody {uid}\r\n"
            )
            .into_bytes()
        };
        let mailbox = FakeMailbox::new()
            .with_inbox(42, 100, 3)
            .with_uids([95, 96, 97])
            .with_body(95, body(95))
            .with_body(96, body(96))
            .with_body(97, body(97));
        let (state, mut rx) =
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

        // First sync: establishes the baseline and must start the history import.
        let queued = poll_once(&state, account_id).await.unwrap();
        assert_eq!(
            queued, 0,
            "the poll itself queues nothing; the backfill imports"
        );

        // The existing mail must arrive on the ingest channel via the backfill the
        // first sync starts — collected with a timeout so a miss fails fast.
        let mut got = std::collections::BTreeSet::new();
        while got.len() < 3 {
            match tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await {
                Ok(Some(raw)) => {
                    got.insert(raw.imap_uid);
                }
                _ => break,
            }
        }
        assert_eq!(
            got.into_iter().collect::<Vec<_>>(),
            vec![95, 96, 97],
            "a newly added mailbox must import its existing mail on first sync"
        );
    }

    /// The secondary pass discovers folders via SPECIAL-USE, then fetches SENT and
    /// JUNK (tagging each body with its local folder) while leaving INBOX to
    /// `poll_once`.
    #[tokio::test]
    async fn poll_secondary_folders_fetches_sent_and_junk_with_tags() {
        use crate::net::fakes::{net_with_imap, FakeImapFactory, FakeMailbox};
        use crate::net::FolderRole;

        let account_id = "5f2d6a1e-0000-4000-8000-0000000000d1";
        let mailbox = FakeMailbox::new()
            .with_folders([
                ("INBOX", FolderRole::Inbox),
                ("[Gmail]/Sent Mail", FolderRole::Sent),
                ("[Gmail]/Spam", FolderRole::Junk),
            ])
            .with_folder_status("[Gmail]/Sent Mail", 10, 6, 1)
            .with_folder_uids("[Gmail]/Sent Mail", [5])
            .with_folder_body(
                "[Gmail]/Sent Mail",
                5,
                b"From: me@x.com\r\nSubject: S\r\nMessage-ID: <s@x>\r\n\r\nbody".to_vec(),
            )
            .with_folder_status("[Gmail]/Spam", 11, 4, 1)
            .with_folder_uids("[Gmail]/Spam", [3])
            .with_folder_body(
                "[Gmail]/Spam",
                3,
                b"From: spammer@x.com\r\nSubject: J\r\nMessage-ID: <j@x>\r\n\r\nbody".to_vec(),
            );
        let (state, mut rx) =
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

        let queued = poll_secondary_folders(&state, account_id).await.unwrap();
        assert_eq!(
            queued, 2,
            "one SENT + one JUNK body queued; INBOX is skipped"
        );

        // Both RawMails arrive tagged with their local folder.
        let mut tags = std::collections::BTreeSet::new();
        for _ in 0..2 {
            let raw = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .expect("a secondary body should arrive")
                .expect("channel open");
            tags.insert(raw.folder);
        }
        assert!(tags.contains("SENT"));
        assert!(tags.contains("JUNK"));
    }
}
