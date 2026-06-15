//! `SyncStateRepo` — the SINGLE writer for `sync_state` (T021 §6).
//!
//! Every per-account sync bookmark / health mutation goes through here; writing
//! `sync_state` directly from anywhere else is an architecture violation.

use super::{map_sqlx_err, Db};
use crate::error::{AppError, AppResult};
use crate::types::SyncState;
use crate::util::now_unix;

/// What one successful `poll_once` learned (T021 §3).
#[derive(Debug, Clone, Copy, Default)]
pub struct SyncOutcome {
    pub inbox_uid_validity: Option<i64>,
    pub inbox_uid_next: Option<i64>,
    /// Newly persisted mail count to add to the running total.
    pub new_mails: u32,
}

#[derive(sqlx::FromRow)]
struct SyncStateRow {
    account_id: String,
    last_sync_at: Option<i64>,
    last_sync_result: Option<String>,
    consecutive_errors: i64,
    backoff_until: Option<i64>,
    inbox_uid_validity: Option<i64>,
    inbox_uid_next: Option<i64>,
    full_sync_required: i64,
    total_mails_synced: i64,
    updated_at: i64,
}

impl From<SyncStateRow> for SyncState {
    fn from(r: SyncStateRow) -> Self {
        SyncState {
            account_id: r.account_id,
            last_sync_at: r.last_sync_at,
            last_sync_result: r.last_sync_result,
            consecutive_errors: r.consecutive_errors as u32,
            backoff_until: r.backoff_until,
            inbox_uid_validity: r.inbox_uid_validity,
            inbox_uid_next: r.inbox_uid_next,
            full_sync_required: r.full_sync_required != 0,
            total_mails_synced: r.total_mails_synced as u32,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Clone)]
pub struct SyncStateRepo<'a> {
    db: &'a Db,
}

impl<'a> SyncStateRepo<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db }
    }

    pub async fn get(&self, account_id: &str) -> AppResult<SyncState> {
        let row: Option<SyncStateRow> = sqlx::query_as(
            "SELECT account_id, last_sync_at, last_sync_result, consecutive_errors, backoff_until, \
                 inbox_uid_validity, inbox_uid_next, full_sync_required, total_mails_synced, \
                 updated_at FROM sync_state WHERE account_id = ?",
        )
        .bind(account_id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        row.map(SyncState::from).ok_or(AppError::NotFound)
    }

    /// Ensure a row exists (idempotent — `create_account` already inserts one).
    pub async fn ensure(&self, account_id: &str) -> AppResult<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO sync_state (account_id, full_sync_required, total_mails_synced, \
                 consecutive_errors, updated_at) VALUES (?, 1, 0, 0, ?)",
        )
        .bind(account_id)
        .bind(now_unix())
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    /// Record a clean poll: advance the UID cursor, clear errors/backoff, bump the
    /// running total. Single statement = single write (T021 §6).
    pub async fn update_after_poll(&self, account_id: &str, o: SyncOutcome) -> AppResult<()> {
        let now = now_unix();
        sqlx::query(
            "UPDATE sync_state SET last_sync_at = ?, last_sync_result = 'ok', \
                 consecutive_errors = 0, backoff_until = NULL, \
                 inbox_uid_validity = COALESCE(?, inbox_uid_validity), \
                 inbox_uid_next = COALESCE(?, inbox_uid_next), \
                 total_mails_synced = total_mails_synced + ?, updated_at = ? \
             WHERE account_id = ?",
        )
        .bind(now)
        .bind(o.inbox_uid_validity)
        .bind(o.inbox_uid_next)
        .bind(o.new_mails as i64)
        .bind(now)
        .bind(account_id)
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    /// Apply a network-error backoff (T021 §3): bump consecutive errors, set the
    /// retry-after watermark, flag the result as a network error.
    pub async fn update_backoff(
        &self,
        account_id: &str,
        consecutive: u32,
        until: i64,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE sync_state SET last_sync_result = 'network_error', consecutive_errors = ?, \
                 backoff_until = ?, updated_at = ? WHERE account_id = ?",
        )
        .bind(consecutive as i64)
        .bind(until)
        .bind(now_unix())
        .bind(account_id)
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn clear_backoff(&self, account_id: &str) -> AppResult<()> {
        sqlx::query(
            "UPDATE sync_state SET consecutive_errors = 0, backoff_until = NULL, updated_at = ? \
             WHERE account_id = ?",
        )
        .bind(now_unix())
        .bind(account_id)
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    /// `UIDVALIDITY` changed: force a full resync and drop the now-meaningless
    /// UID cursor (T021 §6).
    pub async fn flag_uid_validity_change(
        &self,
        account_id: &str,
        new_validity: i64,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE sync_state SET full_sync_required = 1, inbox_uid_next = NULL, \
                 inbox_uid_validity = ?, last_sync_result = 'partial', updated_at = ? \
             WHERE account_id = ?",
        )
        .bind(new_validity)
        .bind(now_unix())
        .bind(account_id)
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn clear_full_sync_required(&self, account_id: &str) -> AppResult<()> {
        sqlx::query(
            "UPDATE sync_state SET full_sync_required = 0, updated_at = ? WHERE account_id = ?",
        )
        .bind(now_unix())
        .bind(account_id)
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

    #[tokio::test]
    async fn poll_then_backoff_then_clear() {
        let db = db_with_account().await;
        let repo = SyncStateRepo::new(&db);

        repo.update_after_poll(
            "a1",
            SyncOutcome {
                inbox_uid_validity: Some(42),
                inbox_uid_next: Some(101),
                new_mails: 5,
            },
        )
        .await
        .unwrap();
        let s = repo.get("a1").await.unwrap();
        assert_eq!(s.inbox_uid_next, Some(101));
        assert_eq!(s.total_mails_synced, 5);
        assert_eq!(s.last_sync_result.as_deref(), Some("ok"));

        repo.update_backoff("a1", 2, 9_999).await.unwrap();
        let s = repo.get("a1").await.unwrap();
        assert_eq!(s.consecutive_errors, 2);
        assert_eq!(s.backoff_until, Some(9_999));

        repo.clear_backoff("a1").await.unwrap();
        assert_eq!(repo.get("a1").await.unwrap().consecutive_errors, 0);
    }
}
