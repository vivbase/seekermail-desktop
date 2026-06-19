//! SQLite storage layer + forward-only migrations (T005, A3).
//!
//! `seekermail.db` (SQLite) is the source of truth (01 Overview). This module owns
//! the connection pool, applies the per-connection PRAGMAs from `01` on every new
//! connection, and runs the embedded `migrations/NNN_*.sql` forward-only via
//! `sqlx::migrate!()`. The frozen schema lands in one shot from `001_init.sql`.
//!
//! No business SQL lives here yet — v0.1 only needs the tables to exist; later
//! feature cards add repositories that share this pool (03 §4).

use std::path::Path;
#[cfg(test)]
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

use crate::error::{AppError, AppResult};

// Repositories sharing this pool (03 §4). Each owns the SQL for one table family;
// no business SQL lives in `mod.rs` itself.
pub mod account_repo;
pub mod attachment_repo;
pub mod backfill_repo;
pub mod blob;
pub mod draft_repo;
pub mod facade;
pub mod identity_repo;
pub mod im_repo;
pub mod mail_repo;
pub mod mail_writer;
pub mod query_repo;
pub mod risk_event_repo;
pub mod settings_repo;
pub mod sync_state_repo;

pub use account_repo::AccountRepo;
pub use attachment_repo::AttachmentRepo;
pub use backfill_repo::BackfillRepo;
pub use blob::DiskBlobStore;
pub use facade::StorageFacade;
pub use identity_repo::IdentityRepo;
pub use mail_repo::MailRepo;
pub use settings_repo::SettingRepo;
pub use sync_state_repo::SyncStateRepo;

/// Embedded migrations, compiled into the binary from `./migrations` and applied
/// at startup. Forward-only — new schema is always a new `NNN_*.sql`, never an
/// edit to `001_init.sql`.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// The shared SQLite handle (sqlx pool). Cloneable; holds an `Arc` internally.
#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    /// Open (creating if needed) the database at `path` and apply the per-
    /// connection PRAGMAs from `01`: WAL, `foreign_keys=ON`, `busy_timeout=5000`,
    /// `temp_store=MEMORY`, `mmap_size`, `cache_size`.
    pub async fn connect(path: &Path) -> AppResult<Self> {
        let connect_options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_millis(5000))
            .pragma("temp_store", "MEMORY")
            .pragma("mmap_size", "134217728")
            .pragma("cache_size", "-8000");

        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(connect_options)
            .await
            .map_err(map_sqlx_err)?;

        Ok(Self { pool })
    }

    /// In-memory database for tests (PRAGMAs applied the same way).
    #[cfg(test)]
    pub async fn connect_in_memory() -> AppResult<Self> {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .map_err(map_sqlx_err)?
            .foreign_keys(true)
            .busy_timeout(Duration::from_millis(5000));
        // A single connection keeps the shared in-memory db alive for the test.
        // Reaping is disabled: a `:memory:` connection IS the database, so the
        // pool must never retire it — and tests on a paused tokio clock
        // (`start_paused`) jump virtual time far past the default idle/lifetime
        // windows, which would otherwise close it mid-test (T067 §8).
        // `test_before_acquire(false)`: the pre-acquire ping is a round-trip to
        // the connection's worker thread; under a paused clock the runtime can
        // auto-advance to the acquire deadline before that round-trip lands,
        // failing perfectly healthy acquires with PoolTimedOut. With the ping
        // off, acquiring the idle connection completes without an await the
        // virtual clock can race.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .test_before_acquire(false)
            .connect_with(opts)
            .await
            .map_err(map_sqlx_err)?;
        Ok(Self { pool })
    }

    /// Apply all pending migrations. Idempotent: sqlx tracks applied versions in
    /// `_sqlx_migrations`, so a second run is a no-op. Maps failures to
    /// `DB_MIGRATION_FAILED`.
    pub async fn run_migrations(&self) -> AppResult<()> {
        MIGRATOR
            .run(&self.pool)
            .await
            .map_err(|e| AppError::DbMigration(e.to_string()))
    }

    /// Cheap storage health probe (used by the v0.1 smoke gate, T012). Returns the
    /// number of accounts (0 on a fresh db).
    pub async fn health_check(&self) -> AppResult<i64> {
        let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM accounts")
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        Ok(count)
    }

    /// Borrow the pool for repositories added by later feature cards.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

/// Translate sqlx errors into the crate error type, distinguishing the few cases
/// the UX cares about (constraint vs not-found vs everything else). Shared by all
/// repositories in this module.
pub(crate) fn map_sqlx_err(err: sqlx::Error) -> AppError {
    match &err {
        sqlx::Error::RowNotFound => AppError::NotFound,
        sqlx::Error::Database(db) if db.is_unique_violation() || db.is_foreign_key_violation() => {
            AppError::DbConstraint(db.message().to_string())
        }
        _ => AppError::Internal(anyhow::anyhow!(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn migrated_db() -> Db {
        let db = Db::connect_in_memory().await.expect("connect");
        db.run_migrations().await.expect("migrate");
        db
    }

    #[tokio::test]
    async fn migrations_create_all_tables() {
        let db = migrated_db().await;
        let names: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .fetch_all(db.pool())
                .await
                .unwrap();
        let tables: Vec<String> = names.into_iter().map(|(n,)| n).collect();
        for expected in [
            "accounts",
            "threads",
            "mails",
            "attachments",
            "contacts",
            "ai_drafts",
            "ai_decisions",
            "risk_events",
            "pending_queries",
            "sync_state",
            "search_history",
            "saved_searches",
            "app_settings",
            "account_ai_settings",
        ] {
            assert!(
                tables.contains(&expected.to_string()),
                "missing table {expected}"
            );
        }
        // FTS5 virtual table is present too.
        assert!(
            tables.contains(&"mails_fts".to_string()),
            "missing mails_fts"
        );
    }

    #[tokio::test]
    async fn migrations_are_idempotent() {
        let db = migrated_db().await;
        // Running again must not error or double-apply.
        db.run_migrations()
            .await
            .expect("second migrate is a no-op");
        assert_eq!(db.health_check().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn foreign_keys_are_enforced() {
        let db = migrated_db().await;
        // Inserting a thread referencing a non-existent account must be rejected,
        // proving foreign_keys=ON took effect per-connection.
        let res = sqlx::query(
            "INSERT INTO threads (id, account_id, subject, participants, latest_date, created_at, updated_at)
             VALUES ('t1', 'no-such-account', 'Hi', '[]', 0, 0, 0)",
        )
        .execute(db.pool())
        .await;
        assert!(res.is_err(), "FK violation must be rejected");
    }

    #[tokio::test]
    async fn seed_app_settings_present() {
        let db = migrated_db().await;
        let (lang,): (String,) =
            sqlx::query_as("SELECT value FROM app_settings WHERE key = 'ui.language'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(lang, "\"en\"");
    }
}
