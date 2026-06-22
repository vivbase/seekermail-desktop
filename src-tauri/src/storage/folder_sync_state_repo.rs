//! `FolderSyncStateRepo` — per-folder IMAP sync cursors (migration 018).
//!
//! Multi-folder fetch (SENT / JUNK / TRASH alongside INBOX) needs a UID cursor
//! per `(account, folder)`, because IMAP `UIDVALIDITY` / `UIDNEXT` are per-folder.
//! Account-level health (errors, backoff, auth) stays in [`super::SyncStateRepo`];
//! this repo owns only the per-folder cursor and its running synced total. Every
//! mutation is a single statement, mirroring `SyncStateRepo`'s single-writer rule.

use super::{map_sqlx_err, Db};
use crate::error::{AppError, AppResult};
use crate::util::now_unix;

/// One per-folder cursor row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderSyncState {
    pub account_id: String,
    pub folder: String,
    pub uid_validity: Option<i64>,
    pub uid_next: Option<i64>,
    pub full_sync_required: bool,
    pub total_mails_synced: u32,
    pub last_sync_at: Option<i64>,
    pub updated_at: i64,
}

#[derive(sqlx::FromRow)]
struct FolderSyncStateRow {
    account_id: String,
    folder: String,
    uid_validity: Option<i64>,
    uid_next: Option<i64>,
    full_sync_required: i64,
    total_mails_synced: i64,
    last_sync_at: Option<i64>,
    updated_at: i64,
}

impl From<FolderSyncStateRow> for FolderSyncState {
    fn from(r: FolderSyncStateRow) -> Self {
        FolderSyncState {
            account_id: r.account_id,
            folder: r.folder,
            uid_validity: r.uid_validity,
            uid_next: r.uid_next,
            full_sync_required: r.full_sync_required != 0,
            total_mails_synced: r.total_mails_synced as u32,
            last_sync_at: r.last_sync_at,
            updated_at: r.updated_at,
        }
    }
}

/// What one successful per-folder poll learned (mirrors `sync_state_repo::SyncOutcome`).
#[derive(Debug, Clone, Copy, Default)]
pub struct FolderSyncOutcome {
    pub uid_validity: Option<i64>,
    pub uid_next: Option<i64>,
    /// Newly persisted mail count to add to the running per-folder total.
    pub new_mails: u32,
}

const COLS: &str = "account_id, folder, uid_validity, uid_next, full_sync_required, \
     total_mails_synced, last_sync_at, updated_at";

#[derive(Clone)]
pub struct FolderSyncStateRepo<'a> {
    db: &'a Db,
}

impl<'a> FolderSyncStateRepo<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Ensure a cursor row exists for this `(account, folder)`. Idempotent — the
    /// scheduler calls it before the first poll of a newly discovered folder.
    pub async fn ensure(&self, account_id: &str, folder: &str) -> AppResult<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO folder_sync_state \
                 (account_id, folder, full_sync_required, total_mails_synced, updated_at) \
             VALUES (?, ?, 1, 0, ?)",
        )
        .bind(account_id)
        .bind(folder)
        .bind(now_unix())
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    /// The cursor for one folder, or [`AppError::NotFound`] if never seen.
    pub async fn get(&self, account_id: &str, folder: &str) -> AppResult<FolderSyncState> {
        self.get_opt(account_id, folder)
            .await?
            .ok_or(AppError::NotFound)
    }

    pub async fn get_opt(
        &self,
        account_id: &str,
        folder: &str,
    ) -> AppResult<Option<FolderSyncState>> {
        let sql =
            format!("SELECT {COLS} FROM folder_sync_state WHERE account_id = ? AND folder = ?");
        let row: Option<FolderSyncStateRow> = sqlx::query_as(&sql)
            .bind(account_id)
            .bind(folder)
            .fetch_optional(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(row.map(FolderSyncState::from))
    }

    /// Every folder cursor for an account, ordered by folder name (drives the
    /// multi-folder scheduler).
    pub async fn list(&self, account_id: &str) -> AppResult<Vec<FolderSyncState>> {
        let sql =
            format!("SELECT {COLS} FROM folder_sync_state WHERE account_id = ? ORDER BY folder");
        let rows: Vec<FolderSyncStateRow> = sqlx::query_as(&sql)
            .bind(account_id)
            .fetch_all(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(rows.into_iter().map(FolderSyncState::from).collect())
    }

    /// Record a clean poll for one folder: advance the UID cursor and bump the
    /// running total. Single statement = single write.
    pub async fn update_after_poll(
        &self,
        account_id: &str,
        folder: &str,
        o: FolderSyncOutcome,
    ) -> AppResult<()> {
        let now = now_unix();
        sqlx::query(
            "UPDATE folder_sync_state SET \
                 uid_validity = COALESCE(?, uid_validity), \
                 uid_next = COALESCE(?, uid_next), \
                 total_mails_synced = total_mails_synced + ?, \
                 last_sync_at = ?, updated_at = ? \
             WHERE account_id = ? AND folder = ?",
        )
        .bind(o.uid_validity)
        .bind(o.uid_next)
        .bind(o.new_mails as i64)
        .bind(now)
        .bind(now)
        .bind(account_id)
        .bind(folder)
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    /// `UIDVALIDITY` changed for one folder: force a full resync and drop the
    /// now-meaningless UID cursor (mirrors the INBOX path in `SyncStateRepo`).
    pub async fn flag_uid_validity_change(
        &self,
        account_id: &str,
        folder: &str,
        new_validity: i64,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE folder_sync_state SET full_sync_required = 1, uid_next = NULL, \
                 uid_validity = ?, updated_at = ? WHERE account_id = ? AND folder = ?",
        )
        .bind(new_validity)
        .bind(now_unix())
        .bind(account_id)
        .bind(folder)
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    /// Clear the full-resync flag once the folder's backfill has caught up.
    pub async fn clear_full_sync_required(&self, account_id: &str, folder: &str) -> AppResult<()> {
        sqlx::query(
            "UPDATE folder_sync_state SET full_sync_required = 0, updated_at = ? \
             WHERE account_id = ? AND folder = ?",
        )
        .bind(now_unix())
        .bind(account_id)
        .bind(folder)
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::account_repo::{AccountRepo, NewAccount};

    async fn db_with_account() -> Db {
        let db = Db::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        AccountRepo::new(&db)
            .create(&NewAccount {
                id: "a1".into(),
                email: "a@x.com".into(),
                display_name: "A".into(),
                provider: "imap".into(),
                imap_host: None,
                imap_port: 993,
                smtp_host: None,
                smtp_port: 587,
                color_token: "slate".into(),
                badge_label: "W".into(),
                role_type: "work".into(),
                role_description: None,
                auth_level: 1,
            })
            .await
            .unwrap();
        db
    }

    /// A folder cursor starts at its "never synced" defaults and `ensure` is
    /// idempotent (a second call does not reset the row).
    #[tokio::test]
    async fn ensure_is_idempotent() {
        let db = db_with_account().await;
        let repo = FolderSyncStateRepo::new(&db);

        repo.ensure("a1", "SENT").await.unwrap();
        repo.update_after_poll(
            "a1",
            "SENT",
            FolderSyncOutcome {
                uid_validity: Some(9),
                uid_next: Some(50),
                new_mails: 3,
            },
        )
        .await
        .unwrap();

        // Second ensure must not clobber the advanced cursor.
        repo.ensure("a1", "SENT").await.unwrap();
        let s = repo.get("a1", "SENT").await.unwrap();
        assert_eq!(s.uid_next, Some(50));
        assert_eq!(s.total_mails_synced, 3);
    }

    /// Advancing one folder's cursor must not touch another folder's — the whole
    /// point of a per-folder cursor.
    #[tokio::test]
    async fn poll_advances_only_that_folder() {
        let db = db_with_account().await;
        let repo = FolderSyncStateRepo::new(&db);
        repo.ensure("a1", "SENT").await.unwrap();
        repo.ensure("a1", "JUNK").await.unwrap();

        repo.update_after_poll(
            "a1",
            "SENT",
            FolderSyncOutcome {
                uid_validity: Some(7),
                uid_next: Some(42),
                new_mails: 5,
            },
        )
        .await
        .unwrap();

        let sent = repo.get("a1", "SENT").await.unwrap();
        assert_eq!(sent.uid_validity, Some(7));
        assert_eq!(sent.uid_next, Some(42));
        assert_eq!(sent.total_mails_synced, 5);

        let junk = repo.get("a1", "JUNK").await.unwrap();
        assert_eq!(junk.uid_next, None, "JUNK cursor must be untouched");
        assert_eq!(junk.total_mails_synced, 0);
    }

    /// A `UIDVALIDITY` change drops the cursor and flags a full resync for that
    /// folder only.
    #[tokio::test]
    async fn uid_validity_change_drops_cursor_and_flags_resync() {
        let db = db_with_account().await;
        let repo = FolderSyncStateRepo::new(&db);
        repo.ensure("a1", "SENT").await.unwrap();
        repo.update_after_poll(
            "a1",
            "SENT",
            FolderSyncOutcome {
                uid_validity: Some(1),
                uid_next: Some(100),
                new_mails: 0,
            },
        )
        .await
        .unwrap();

        repo.flag_uid_validity_change("a1", "SENT", 2)
            .await
            .unwrap();
        let s = repo.get("a1", "SENT").await.unwrap();
        assert_eq!(s.uid_validity, Some(2));
        assert_eq!(s.uid_next, None, "cursor dropped on UIDVALIDITY change");
        assert!(s.full_sync_required);
    }

    /// `list` returns every ensured folder for the account.
    #[tokio::test]
    async fn list_returns_ensured_folders() {
        let db = db_with_account().await;
        let repo = FolderSyncStateRepo::new(&db);
        repo.ensure("a1", "SENT").await.unwrap();
        repo.ensure("a1", "TRASH").await.unwrap();

        let folders: Vec<String> = repo
            .list("a1")
            .await
            .unwrap()
            .into_iter()
            .map(|f| f.folder)
            .collect();
        assert!(folders.contains(&"SENT".to_string()));
        assert!(folders.contains(&"TRASH".to_string()));
    }
}
