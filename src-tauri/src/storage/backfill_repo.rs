//! `BackfillRepo` — CRUD over `backfill_state` (T022, migration 003).
//!
//! Drives the resumable history backfill: the `last_uid_fetched` cursor is the
//! resume point, `status` the lifecycle, `fetched_count`/`total_uid_count` the
//! progress fraction.

use super::{map_sqlx_err, Db};
use crate::error::{AppError, AppResult};
use crate::types::BackfillStatus;
use crate::util::now_unix;

#[derive(sqlx::FromRow)]
struct BackfillRow {
    account_id: String,
    status: String,
    depth_months: Option<i64>,
    boundary_date: Option<i64>,
    last_uid_fetched: Option<i64>,
    total_uid_count: Option<i64>,
    fetched_count: i64,
    started_at: Option<i64>,
    paused_at: Option<i64>,
    completed_at: Option<i64>,
    error_message: Option<String>,
    updated_at: i64,
}

impl From<BackfillRow> for BackfillStatus {
    fn from(r: BackfillRow) -> Self {
        BackfillStatus {
            account_id: r.account_id,
            status: r.status,
            depth_months: r.depth_months.map(|m| m as u32),
            boundary_date: r.boundary_date,
            last_uid_fetched: r.last_uid_fetched,
            total_uid_count: r.total_uid_count.map(|c| c as u32),
            fetched_count: r.fetched_count as u32,
            started_at: r.started_at,
            paused_at: r.paused_at,
            completed_at: r.completed_at,
            error_message: r.error_message,
            updated_at: r.updated_at,
        }
    }
}

const COLS: &str = "account_id, status, depth_months, boundary_date, last_uid_fetched, \
     total_uid_count, fetched_count, started_at, paused_at, completed_at, error_message, updated_at";

#[derive(Clone)]
pub struct BackfillRepo<'a> {
    db: &'a Db,
}

impl<'a> BackfillRepo<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db }
    }

    pub async fn get(&self, account_id: &str) -> AppResult<BackfillStatus> {
        let sql = format!("SELECT {COLS} FROM backfill_state WHERE account_id = ?");
        let row: Option<BackfillRow> = sqlx::query_as(&sql)
            .bind(account_id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        row.map(BackfillStatus::from).ok_or(AppError::NotFound)
    }

    pub async fn get_opt(&self, account_id: &str) -> AppResult<Option<BackfillStatus>> {
        match self.get(account_id).await {
            Ok(s) => Ok(Some(s)),
            Err(AppError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Begin (or restart) a backfill: status=running, totals reset, cursor cleared.
    pub async fn start(
        &self,
        account_id: &str,
        depth_months: Option<u32>,
        boundary_date: Option<i64>,
        total_uid_count: u32,
    ) -> AppResult<()> {
        let now = now_unix();
        sqlx::query(
            "INSERT INTO backfill_state (account_id, status, depth_months, boundary_date, \
                 last_uid_fetched, total_uid_count, fetched_count, started_at, updated_at) \
             VALUES (?, 'running', ?, ?, NULL, ?, 0, ?, ?) \
             ON CONFLICT(account_id) DO UPDATE SET status='running', depth_months=excluded.depth_months, \
                 boundary_date=excluded.boundary_date, total_uid_count=excluded.total_uid_count, \
                 last_uid_fetched=NULL, fetched_count=0, started_at=excluded.started_at, \
                 paused_at=NULL, completed_at=NULL, error_message=NULL, updated_at=excluded.updated_at",
        )
        .bind(account_id)
        .bind(depth_months.map(|m| m as i64))
        .bind(boundary_date)
        .bind(total_uid_count as i64)
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    /// Advance the resume cursor after a batch is persisted.
    pub async fn advance(
        &self,
        account_id: &str,
        last_uid_fetched: i64,
        fetched_count: u32,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE backfill_state SET last_uid_fetched = ?, fetched_count = ?, status = 'running', \
                 updated_at = ? WHERE account_id = ?",
        )
        .bind(last_uid_fetched)
        .bind(fetched_count as i64)
        .bind(now_unix())
        .bind(account_id)
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn set_paused(&self, account_id: &str) -> AppResult<()> {
        sqlx::query(
            "UPDATE backfill_state SET status = 'paused', paused_at = ?, updated_at = ? \
             WHERE account_id = ?",
        )
        .bind(now_unix())
        .bind(now_unix())
        .bind(account_id)
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn set_completed(&self, account_id: &str) -> AppResult<()> {
        sqlx::query(
            "UPDATE backfill_state SET status = 'completed', completed_at = ?, updated_at = ? \
             WHERE account_id = ?",
        )
        .bind(now_unix())
        .bind(now_unix())
        .bind(account_id)
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn set_error(&self, account_id: &str, message: &str) -> AppResult<()> {
        sqlx::query(
            "UPDATE backfill_state SET status = 'error', error_message = ?, updated_at = ? \
             WHERE account_id = ?",
        )
        .bind(message)
        .bind(now_unix())
        .bind(account_id)
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    /// Account ids whose backfill should auto-resume on startup (T022 §3): any row
    /// not already completed or explicitly paused.
    pub async fn list_resumable(&self) -> AppResult<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT account_id FROM backfill_state WHERE status NOT IN ('completed', 'paused')",
        )
        .fetch_all(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
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
    async fn start_advance_pause_resume() {
        let db = db_with_account().await;
        let repo = BackfillRepo::new(&db);

        repo.start("a1", Some(3), Some(1000), 200).await.unwrap();
        let s = repo.get("a1").await.unwrap();
        assert_eq!(s.status, "running");
        assert_eq!(s.total_uid_count, Some(200));

        repo.advance("a1", 150, 50).await.unwrap();
        assert_eq!(repo.get("a1").await.unwrap().last_uid_fetched, Some(150));

        repo.set_paused("a1").await.unwrap();
        assert_eq!(repo.get("a1").await.unwrap().status, "paused");
        // Paused accounts are not auto-resumed.
        assert!(repo.list_resumable().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn cascades_on_account_delete() {
        let db = db_with_account().await;
        let repo = BackfillRepo::new(&db);
        repo.start("a1", None, None, 10).await.unwrap();
        AccountRepo::new(&db).delete("a1").await.unwrap();
        assert!(repo.get_opt("a1").await.unwrap().is_none());
    }
}
