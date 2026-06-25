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
pub mod folder_sync_state_repo;
pub mod identity_repo;
pub mod im_repo;
pub mod inbox_digest_repo;
pub mod mail_repo;
pub mod mail_writer;
pub mod outbound_op_repo;
pub mod query_repo;
pub mod risk_event_repo;
pub mod settings_repo;
pub mod sync_state_repo;
pub mod thread_summary_repo;

pub use account_repo::AccountRepo;
pub use attachment_repo::AttachmentRepo;
pub use backfill_repo::BackfillRepo;
pub use blob::DiskBlobStore;
pub use facade::StorageFacade;
pub use folder_sync_state_repo::{FolderSyncOutcome, FolderSyncState, FolderSyncStateRepo};
pub use identity_repo::IdentityRepo;
pub use mail_repo::MailRepo;
pub use outbound_op_repo::{OutboundOp, OutboundOpKind, OutboundOpRepo};
pub use settings_repo::SettingRepo;
pub use sync_state_repo::SyncStateRepo;

/// Embedded migrations, compiled into the binary from `./migrations` and applied
/// at startup. Forward-only — new schema is always a new `NNN_*.sql`, never an
/// edit to `001_init.sql`.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// The external-content FTS5 indexes kept in sync with `mails` / `attachments`.
/// Fixed schema identifiers — safe to interpolate into repair SQL.
const FTS_TABLES: [&str; 2] = ["mails_fts", "attachments_fts"];

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

    /// Startup self-heal for corrupt FTS5 shadow indexes (`SQLITE_CORRUPT_VTAB`,
    /// code 267).
    ///
    /// Both full-text indexes (`mails_fts`, `attachments_fts`) are external-content
    /// FTS5 tables derived from `mails` / `attachments`. An unclean shutdown can
    /// leave a shadow index "malformed"; because a cascading `DELETE` on `mails`
    /// fires the FTS sync triggers, that corruption makes **account removal** (and
    /// sync) fail with "database disk image is malformed". We run the cheap FTS
    /// `'integrity-check'` and, only when it reports corruption, repair the index.
    /// Best-effort by design: a heal failure is logged, never fatal — startup must
    /// still proceed even if an index is unrepairable (search degrades; the app
    /// still runs).
    ///
    /// IMPORTANT: `'integrity-check'` does **not** catch every corruption that
    /// aborts the delete-trigger write path — an index can pass the check yet still
    /// fail the cascading `DELETE` with code 267. Account removal therefore does
    /// **not** rely on this cheap check; it calls [`Self::repair_fts_indexes_forced`]
    /// directly on failure (which rebuilds unconditionally and propagates errors).
    pub async fn heal_fts_indexes(&self) -> AppResult<()> {
        for table in FTS_TABLES {
            // Fixed schema identifiers, never user input — safe to interpolate.
            let check = sqlx::query(&format!(
                "INSERT INTO {table}({table}) VALUES('integrity-check')"
            ))
            .execute(&self.pool)
            .await;
            if let Err(err) = check {
                tracing::warn!(table, error = %err, "FTS integrity-check failed; repairing index");
                match self.repair_fts_table(table).await {
                    Ok(()) => tracing::info!(table, "repaired corrupt FTS index"),
                    Err(err) => {
                        tracing::error!(table, error = %err, "FTS index repair failed; continuing")
                    }
                }
            }
        }
        Ok(())
    }

    /// Forcefully repair every FTS5 index, regardless of what `'integrity-check'`
    /// reports, and **propagate** any failure.
    ///
    /// Used by the account-delete retry: a `DELETE` that already failed with
    /// "database disk image is malformed" is itself proof of corruption that the
    /// cheap check can miss, so the only safe move is to rebuild unconditionally
    /// and let the caller know whether the retry can now succeed.
    pub async fn repair_fts_indexes_forced(&self) -> AppResult<()> {
        for table in FTS_TABLES {
            self.repair_fts_table(table).await?;
        }
        Ok(())
    }

    /// Repair one external-content FTS5 index, escalating until it sticks:
    /// 1. `'rebuild'` — repopulate the shadow index from its content table.
    /// 2. `'delete-all'` then `'rebuild'` — zero the (possibly malformed) shadow
    ///    tables without reading them, then repopulate from content.
    /// 3. `DROP TABLE` + recreate from its stored DDL + `'rebuild'` — discard the
    ///    corrupt shadow tables entirely. Done in one transaction so concurrent
    ///    writers wait on the lock rather than observing a missing table; the sync
    ///    triggers reference the table by name and rebind to the fresh one.
    ///
    /// `table` is a fixed schema identifier (never user input) — safe to interpolate.
    async fn repair_fts_table(&self, table: &str) -> AppResult<()> {
        if self.fts_rebuild(table).await.is_ok() {
            return Ok(());
        }
        // The shadow b-tree may be malformed; `'delete-all'` rewrites it without a
        // content read, which often clears what a plain `'rebuild'` cannot.
        let _ = sqlx::query(&format!(
            "INSERT INTO {table}({table}) VALUES('delete-all')"
        ))
        .execute(&self.pool)
        .await;
        if self.fts_rebuild(table).await.is_ok() {
            return Ok(());
        }
        // Last resort: drop the virtual table (removing its corrupt shadow tables)
        // and recreate it from the DDL SQLite stored at migration time.
        let mut tx = self.pool.begin().await.map_err(map_sqlx_err)?;
        let (ddl,): (String,) =
            sqlx::query_as("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?")
                .bind(table)
                .fetch_one(&mut *tx)
                .await
                .map_err(map_sqlx_err)?;
        sqlx::query(&format!("DROP TABLE {table}"))
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_err)?;
        sqlx::query(&ddl)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_err)?;
        sqlx::query(&format!("INSERT INTO {table}({table}) VALUES('rebuild')"))
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_err)?;
        tx.commit().await.map_err(map_sqlx_err)?;
        Ok(())
    }

    /// Repopulate an external-content FTS5 index from its content table.
    async fn fts_rebuild(&self, table: &str) -> AppResult<()> {
        sqlx::query(&format!("INSERT INTO {table}({table}) VALUES('rebuild')"))
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(map_sqlx_err)
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

    /// Shipped migrations must be **immutable**. sqlx records a checksum for every
    /// applied migration; if a migration's bytes later change, it refuses to run
    /// against a database that applied the old version ("migration N was previously
    /// applied but has been modified") — which crashes existing installs on launch
    /// (the setup hook turns that error into a panic). So a released `NNN_*.sql` may
    /// never be edited or renumbered; new schema is always a new, higher-numbered
    /// file.
    ///
    /// This test freezes each shipped migration's `(version, sha384)`. If it fails
    /// for an existing version, you changed a shipped migration — revert it and add
    /// the change as a NEW migration instead. When you intentionally ADD a migration,
    /// append its `(version, checksum)` below (the failure message prints the value).
    #[test]
    fn shipped_migrations_are_immutable() {
        // (version, lowercase sha384 hex). sqlx checksums the migration file bytes.
        let frozen: &[(i64, &str)] = &[
            (1, "f4cf5f575276410b1fcf3e367b327bfe401fa23d1534a0dabf773b4a40328833693664636fee76014d245eb91a5617c2"),
            (2, "409ef81338bf9f7798734e3c41afeeb91377b8105c4fd68615a133996167adb978f8cdbf5d5ecf68f42d49dd5226309e"),
            (3, "b79a439189e43eaa552c45392a8836bca0d302be0e096615de0547e057d3d2e103d4768675a6f39f178fcbdbb435a928"),
            (4, "002315e0b33ee6f5ce3bdde5f42c80faf8b5554c03e29d0c051916f10ba1c0503f906857177ebfca54395bec6e70bf83"),
            (5, "dc48d4490ee60fa923a33b0a148a022410a5f9b505cca0497d574d66bd0f067720e1cde4571f036e60f9c98fd5b257d5"),
            (6, "47c4c414656e91d32ee740482e4bbadbfbe0e861923472260448a15dc885f03a22abeacf4145a089d181ba4ffa9b324b"),
            (7, "3f275cfa0bb71cfbf5d0384c874140c27ccb7b28b112dad57ce0662067d00ad8bd3741de86f2651e469619c2d56f9395"),
            (8, "bb0b0d2501e217f140382b604afcb28c6ffa3ba2514bd5592c692ad3862316b096f1332bcd20a0f8fc5cc90f2f7dbb09"),
            (9, "fbbc5394cb9486061a91483f6a3363a73a08c9307300355a7ed6d78db21caee22e3da8f7645a670928fcc60090012f8a"),
            (10, "5cc0aeaeaaf5f2a810ebc3bfee268c5d54eefa2a12e4b1361e1671f6059b0335cbbc22e0c340088114f84005e7d253db"),
            (11, "ffe12c7397705aa8d41d3c554e20441db29e2563197c10b3ba6608c8370d4a36cbbfe9e19ca75f7fc295b251ecdc44b3"),
            (12, "353d22f87d2d7e77bf233df2d4c9d352c76733b67062885bbd9117455f2c4b2e59976f4b621b0e2bb82614bb147e6213"),
            (13, "3362b9203c22b225ff303cb921ed4304e72f59d6b48878cd004db2730b3ea853135e3c925ed1420d2a7181f16fd8641e"),
            (14, "56f52ee347182b6fe759c70b6bcf8958ab49acc326f77b7910cd6af82747aa4631e73b633b63459d79044c847b820540"),
            (15, "a259ad5c263aacd467d43731b5a52b5f56d3e17779d8103e008efdde78c920856ad12cda83c73ed34c0054ab95aeada1"),
            (16, "096e2fddb163601da72fa0d860d8aa66a78fb66fc3eae4297cb3b181abb0f4451a139ee82b9ca2d48b9c6cb9405bf77a"),
            (17, "062c6dd56ff838bf6b3bf1dfe6cb7ccc097ee31e5071100ad2596bb70bd2885062d379794f3efb65125e5bb5113107dd"),
            (18, "f0a80edf4f1de99cae64c2a40d6157c467ff810d00a73ba96f9fcbf86ee2908bfcbfced570820af5a65dde05656172b2"),
            (19, "2cd7cae23813cc967f019a55b2750c8b47100706503c9d33e93483e83ec1fb3403abacaaefb8f9c7a6339e8405befa9f"),
            (20, "f0284b536277cca8a522d79afa2309e0919ae74203e7ef4b2165666eb6fee6b958660a3d7c58ecbb42dc2dc53a1e0080"),
            (21, "c1febb6e5d5805f8ef4249c3b4697ab281a859afcfe27d735620a5e4f8a1d2b5e915a6182b03635cc7067c7059784502"),
            (22, "9e0c98971a68a1388dd968e0e0af3c4f49d530e2eb6bf54ae9ae9c3c0a5f75ac83d3737862ccd7dc37710a7afa7f7d9d"),
        ];

        let actual: std::collections::BTreeMap<i64, String> = MIGRATOR
            .iter()
            .filter(|m| !m.migration_type.is_down_migration())
            .map(|m| (m.version, hex_lower(m.checksum.as_ref())))
            .collect();

        // Every frozen migration must still be present with the same checksum.
        for (version, sum) in frozen {
            match actual.get(version) {
                Some(found) => assert_eq!(
                    found, sum,
                    "migration {version} changed — shipped migrations are immutable; \
                     revert it and add a NEW migration instead"
                ),
                None => panic!("frozen migration {version} is missing from the embedded set"),
            }
        }

        // No NEW migration may ship without being frozen here (forces a deliberate
        // append, so a future edit to it is caught too).
        let frozen_versions: std::collections::BTreeSet<i64> =
            frozen.iter().map(|(v, _)| *v).collect();
        for (version, sum) in &actual {
            assert!(
                frozen_versions.contains(version),
                "migration {version} is not frozen — append ({version}, \"{sum}\") to the \
                 frozen list in this test"
            );
        }
    }

    /// Lowercase hex of a byte slice (sqlx stores migration checksums as bytes).
    fn hex_lower(bytes: &[u8]) -> String {
        use std::fmt::Write as _;
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            let _ = write!(s, "{b:02x}");
        }
        s
    }

    async fn migrated_db() -> Db {
        let db = Db::connect_in_memory().await.expect("connect");
        db.run_migrations().await.expect("migrate");
        db
    }

    #[tokio::test]
    async fn heal_fts_indexes_is_noop_on_healthy_db() {
        // Validates the heal SQL and that it never harms a consistent index: a
        // row stays searchable across two heal passes (idempotent, best-effort).
        let db = migrated_db().await;
        let pool = db.pool();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
             created_at, updated_at) VALUES ('a1','a1@x.com','W','imap','slate','W',0,0)",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, from_email, to_addrs, date_sent, \
             date_received, body_text, created_at, updated_at) \
             VALUES ('m1','a1','<1@x>','s@x.y','[]',1,1,'figures attached budget',0,0)",
        )
        .execute(pool)
        .await
        .unwrap();
        db.heal_fts_indexes().await.expect("heal pass 1");
        db.heal_fts_indexes().await.expect("heal pass 2");
        let (n,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM mails_fts WHERE mails_fts MATCH 'budget'")
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(n, 1, "healthy index stays searchable after heal");
    }

    #[tokio::test]
    async fn forced_fts_repair_repopulates_and_is_idempotent() {
        // The forced repair must restore a broken index unconditionally (the path
        // account removal relies on, since `'integrity-check'` can miss the
        // corruption that aborts the delete trigger). We simulate a broken index
        // by emptying it out-of-band, then assert forced repair rebuilds it from
        // the content table — twice, to prove idempotency.
        let db = migrated_db().await;
        let pool = db.pool();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
             created_at, updated_at) VALUES ('a1','a1@x.com','W','imap','slate','W',0,0)",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, from_email, to_addrs, date_sent, \
             date_received, body_text, created_at, updated_at) \
             VALUES ('m1','a1','<1@x>','s@x.y','[]',1,1,'quarterly budget figures',0,0)",
        )
        .execute(pool)
        .await
        .unwrap();

        // Break the index: drop all postings without touching the content table.
        sqlx::query("INSERT INTO mails_fts(mails_fts) VALUES('delete-all')")
            .execute(pool)
            .await
            .unwrap();
        let (before,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM mails_fts WHERE mails_fts MATCH 'budget'")
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(before, 0, "index emptied before repair");

        db.repair_fts_indexes_forced().await.expect("forced repair");
        db.repair_fts_indexes_forced()
            .await
            .expect("forced repair is idempotent");

        let (after,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM mails_fts WHERE mails_fts MATCH 'budget'")
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(after, 1, "forced repair repopulates the index from content");
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
