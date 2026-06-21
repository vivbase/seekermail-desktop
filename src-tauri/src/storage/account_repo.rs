//! `AccountRepo` — all SQL for the `accounts` family (T013, T016, T018).
//!
//! Credentials never appear in any statement here (08 §7): the only secret-bearing
//! field, `CreateAccountParams.password`, is consumed at the command boundary and
//! handed to the Keychain. `create` writes the three rows an account needs —
//! `accounts`, `sync_state`, `account_ai_settings` — in a single transaction
//! (02 §create_account side-effects).

use sqlx::Row;

use super::{map_sqlx_err, Db, SyncStateRepo};
use crate::error::{AppError, AppResult};
use crate::types::{Account, AgentStatus, UpdateAccountParams};
use crate::util::now_unix;

/// Window (seconds) during which a freshly generated draft marks an agent
/// "processing" (T094, F_I2 §4.2).
const PROCESSING_WINDOW_SECS: i64 = 300;

/// Resolved, validated fields for a new account row (built by the service from
/// [`crate::types::CreateAccountParams`] after autodiscover + validation).
#[derive(Debug, Clone)]
pub struct NewAccount {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub provider: String,
    pub imap_host: Option<String>,
    pub imap_port: u16,
    pub smtp_host: Option<String>,
    pub smtp_port: u16,
    pub color_token: String,
    pub badge_label: String,
    pub role_type: String,
    pub role_description: Option<String>,
    pub auth_level: u8,
}

/// DB projection of an account row (the columns the [`Account`] DTO needs).
#[derive(sqlx::FromRow)]
struct AccountRow {
    id: String,
    email: String,
    display_name: String,
    provider: String,
    imap_host: Option<String>,
    imap_port: i64,
    smtp_host: Option<String>,
    smtp_port: i64,
    color_token: String,
    badge_label: String,
    role_type: String,
    role_description: Option<String>,
    auth_level: i64,
    is_primary: i64,
    is_active: i64,
    sync_interval_secs: i64,
    last_synced_at: Option<i64>,
    knowledge_depth_months: Option<i64>,
    created_at: i64,
    updated_at: i64,
}

impl From<AccountRow> for Account {
    fn from(r: AccountRow) -> Self {
        Account {
            id: r.id,
            email: r.email,
            display_name: r.display_name,
            provider: r.provider,
            imap_host: r.imap_host,
            imap_port: r.imap_port as u16,
            smtp_host: r.smtp_host,
            smtp_port: r.smtp_port as u16,
            color_token: r.color_token,
            badge_label: r.badge_label,
            role_type: r.role_type,
            role_description: r.role_description,
            auth_level: r.auth_level as u8,
            is_primary: r.is_primary != 0,
            is_active: r.is_active != 0,
            sync_interval_secs: r.sync_interval_secs as u32,
            last_synced_at: r.last_synced_at,
            knowledge_depth_months: r.knowledge_depth_months.map(|m| m as u32),
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

const SELECT_COLS: &str = "id, email, display_name, provider, imap_host, imap_port, smtp_host, \
     smtp_port, color_token, badge_label, role_type, role_description, auth_level, is_primary, \
     is_active, sync_interval_secs, last_synced_at, knowledge_depth_months, created_at, updated_at";

/// Stateless repository over the shared pool.
#[derive(Clone)]
pub struct AccountRepo<'a> {
    db: &'a Db,
}

impl<'a> AccountRepo<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Every account, primary first then alphabetical by email.
    pub async fn list(&self) -> AppResult<Vec<Account>> {
        let sql = format!("SELECT {SELECT_COLS} FROM accounts ORDER BY is_primary DESC, email ASC");
        let rows: Vec<AccountRow> = sqlx::query_as(&sql)
            .fetch_all(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(rows.into_iter().map(Account::from).collect())
    }

    /// One account by id, `NOT_FOUND` if it does not exist.
    pub async fn get(&self, id: &str) -> AppResult<Account> {
        let sql = format!("SELECT {SELECT_COLS} FROM accounts WHERE id = ?");
        let row: Option<AccountRow> = sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        row.map(Account::from).ok_or(AppError::NotFound)
    }

    /// Total account count. Retained for tests/diagnostics; the last-account delete
    /// guard it once backed was removed in the A6 identity decoupling (analysis/26),
    /// since removing the last mailbox is now a valid state.
    #[allow(dead_code)]
    pub async fn count(&self) -> AppResult<i64> {
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM accounts")
            .fetch_one(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(n)
    }

    /// Insert an account plus its `sync_state` and `account_ai_settings` rows in
    /// one transaction (02 §create_account). The first-ever account becomes the
    /// primary automatically; a duplicate email surfaces as `DB_CONSTRAINT`.
    pub async fn create(&self, a: &NewAccount) -> AppResult<Account> {
        let now = now_unix();
        let mut tx = self.db.pool().begin().await.map_err(map_sqlx_err)?;

        // First-primary rule (T091, F_I1 §3): a new account becomes primary iff
        // the database currently has no primary at all — not merely when it is the
        // first row. This makes `create` self-heal a primary-less database (e.g.
        // after an aborted migration) instead of leaving every account secondary.
        let primary_count: (i64,) =
            sqlx::query_as("SELECT count(*) FROM accounts WHERE is_primary = 1")
                .fetch_one(&mut *tx)
                .await
                .map_err(map_sqlx_err)?;
        let is_primary: i64 = if primary_count.0 == 0 { 1 } else { 0 };

        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, imap_host, imap_port, \
                 smtp_host, smtp_port, color_token, badge_label, role_type, role_description, \
                 auth_level, is_primary, is_active, sync_interval_secs, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, 60, ?, ?)",
        )
        .bind(&a.id)
        .bind(&a.email)
        .bind(&a.display_name)
        .bind(&a.provider)
        .bind(&a.imap_host)
        .bind(a.imap_port as i64)
        .bind(&a.smtp_host)
        .bind(a.smtp_port as i64)
        .bind(&a.color_token)
        .bind(&a.badge_label)
        .bind(&a.role_type)
        .bind(&a.role_description)
        .bind(a.auth_level as i64)
        .bind(is_primary)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_err)?;

        // sync_state: a fresh account needs a full sync.
        sqlx::query(
            "INSERT INTO sync_state (account_id, full_sync_required, total_mails_synced, \
                 consecutive_errors, updated_at) VALUES (?, 1, 0, 0, ?)",
        )
        .bind(&a.id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_err)?;

        // account_ai_settings: all schema defaults; mirror auth_level.
        sqlx::query(
            "INSERT INTO account_ai_settings (account_id, auth_level, updated_at) VALUES (?, ?, ?)",
        )
        .bind(&a.id)
        .bind(a.auth_level as i64)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_err)?;

        tx.commit().await.map_err(map_sqlx_err)?;
        self.get(&a.id).await
    }

    /// Apply a partial update (read-modify-write). `email` is not a field of the
    /// patch, so it is immutable by construction. Handles primary transfer and
    /// refuses to un-set the sole primary account.
    pub async fn update(&self, id: &str, patch: &UpdateAccountParams) -> AppResult<Account> {
        let cur = self.get(id).await?;
        let now = now_unix();
        let mut tx = self.db.pool().begin().await.map_err(map_sqlx_err)?;

        // Primary transfer rules (01 "max one per db").
        let new_primary = match patch.is_primary {
            Some(true) if !cur.is_primary => {
                sqlx::query(
                    "UPDATE accounts SET is_primary = 0, updated_at = ? WHERE is_primary = 1",
                )
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx_err)?;
                true
            }
            Some(false) if cur.is_primary => {
                return Err(AppError::Forbidden(
                    "cannot unset the only primary account".into(),
                ));
            }
            _ => cur.is_primary,
        };

        let display_name = patch.display_name.clone().unwrap_or(cur.display_name);
        let color_token = patch.color_token.clone().unwrap_or(cur.color_token);
        let badge_label = patch.badge_label.clone().unwrap_or(cur.badge_label);
        let role_type = patch.role_type.clone().unwrap_or(cur.role_type);
        let role_description = patch.role_description.clone().or(cur.role_description);
        let auth_level = patch.auth_level.unwrap_or(cur.auth_level);
        let is_active = patch.is_active.unwrap_or(cur.is_active);
        let sync_interval = patch.sync_interval_secs.unwrap_or(cur.sync_interval_secs);
        let imap_host = patch.imap_host.clone().or(cur.imap_host);
        let imap_port = patch.imap_port.unwrap_or(cur.imap_port);
        let smtp_host = patch.smtp_host.clone().or(cur.smtp_host);
        let smtp_port = patch.smtp_port.unwrap_or(cur.smtp_port);

        sqlx::query(
            "UPDATE accounts SET display_name = ?, color_token = ?, badge_label = ?, \
                 role_type = ?, role_description = ?, auth_level = ?, is_primary = ?, \
                 is_active = ?, sync_interval_secs = ?, imap_host = ?, imap_port = ?, \
                 smtp_host = ?, smtp_port = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&display_name)
        .bind(&color_token)
        .bind(&badge_label)
        .bind(&role_type)
        .bind(&role_description)
        .bind(auth_level as i64)
        .bind(new_primary as i64)
        .bind(is_active as i64)
        .bind(sync_interval as i64)
        .bind(&imap_host)
        .bind(imap_port as i64)
        .bind(&smtp_host)
        .bind(smtp_port as i64)
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_err)?;

        tx.commit().await.map_err(map_sqlx_err)?;
        self.get(id).await
    }

    /// Delete an account row. `ON DELETE CASCADE` removes threads/mails/etc. The
    /// "last account" guard lives in the service, above this call.
    pub async fn delete(&self, id: &str) -> AppResult<()> {
        let affected = match self.exec_delete(id).await {
            Ok(rows) => rows,
            Err(_first) => {
                // A corrupt FTS shadow index (`SQLITE_CORRUPT_VTAB`, code 267) makes
                // the cascading `DELETE` on `mails` fail with "database disk image is
                // malformed". The cheap startup `'integrity-check'` can pass while
                // this delete-trigger path still fails, so a plain heal is not
                // enough here — force a full FTS rebuild (escalating to drop+recreate)
                // and retry once, so removing an account can never be permanently
                // blocked by a derived-index corruption.
                tracing::warn!(
                    account_id = id,
                    "account delete failed; repairing FTS and retrying"
                );
                self.db.repair_fts_indexes_forced().await?;
                self.exec_delete(id).await.map_err(map_sqlx_err)?
            }
        };
        if affected == 0 {
            return Err(AppError::NotFound);
        }
        Ok(())
    }

    /// One `DELETE FROM accounts` attempt; returns the rows affected so [`Self::delete`]
    /// can distinguish "not found" (0 rows) from a DB error worth healing + retrying.
    async fn exec_delete(&self, id: &str) -> Result<u64, sqlx::Error> {
        let res = sqlx::query("DELETE FROM accounts WHERE id = ?")
            .bind(id)
            .execute(self.db.pool())
            .await?;
        Ok(res.rows_affected())
    }

    /// Promote one account to primary in a single transaction (T091, F_I1 §4):
    /// clear every `is_primary` flag, then set the target's — so the invariant
    /// "at most one primary" can never be violated mid-write. The target must
    /// exist and be active; otherwise `NOT_FOUND`.
    pub async fn set_primary(&self, id: &str) -> AppResult<Account> {
        let now = now_unix();
        let mut tx = self.db.pool().begin().await.map_err(map_sqlx_err)?;

        // Guard: only an existing, active account can become primary.
        let active: Option<(i64,)> = sqlx::query_as("SELECT is_active FROM accounts WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx_err)?;
        match active {
            Some((is_active,)) if is_active != 0 => {}
            _ => return Err(AppError::NotFound),
        }

        sqlx::query("UPDATE accounts SET is_primary = 0, updated_at = ? WHERE is_primary = 1")
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_err)?;
        sqlx::query("UPDATE accounts SET is_primary = 1, updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_err)?;

        tx.commit().await.map_err(map_sqlx_err)?;
        self.get(id).await
    }

    /// Startup self-heal for the single-primary invariant (T091, F_I1 §6). If the
    /// database has exactly one primary, this is a no-op. Otherwise (0 or ≥2
    /// primaries — corruption, a half-applied migration, a manual edit) it resets
    /// every flag and promotes the earliest-created *active* account. Returns the
    /// number of primary rows found before healing, so the caller can log it.
    pub async fn heal_primary(&self) -> AppResult<i64> {
        let (total,): (i64,) = sqlx::query_as("SELECT count(*) FROM accounts")
            .fetch_one(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        if total == 0 {
            return Ok(1); // Nothing to heal on an empty database.
        }
        let (primary_count,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM accounts WHERE is_primary = 1")
                .fetch_one(self.db.pool())
                .await
                .map_err(map_sqlx_err)?;
        if primary_count == 1 {
            return Ok(1);
        }

        let now = now_unix();
        let mut tx = self.db.pool().begin().await.map_err(map_sqlx_err)?;
        sqlx::query("UPDATE accounts SET is_primary = 0, updated_at = ? WHERE is_primary = 1")
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_err)?;
        // Prefer the earliest active account; fall back to the earliest account
        // overall if none is active, so the invariant always holds afterwards.
        sqlx::query(
            "UPDATE accounts SET is_primary = 1, updated_at = ? WHERE id = (\
                 SELECT id FROM accounts \
                 ORDER BY is_active DESC, created_at ASC, id ASC LIMIT 1\
             )",
        )
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_err)?;
        tx.commit().await.map_err(map_sqlx_err)?;
        Ok(primary_count)
    }

    /// Derive each active account's Agent presence (T094, F_I2 §4.2). Pure
    /// read: `offline` when the last sync hit an auth/network error, `processing`
    /// when a draft was generated within the last 5 minutes, else `idle`. Ordered
    /// primary-first so the UI can render the master agent leftmost.
    pub async fn agent_statuses(&self) -> AppResult<Vec<AgentStatus>> {
        let since = now_unix() - PROCESSING_WINDOW_SECS;
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT a.id, CASE \
                 WHEN ss.last_sync_result IN ('auth_error','network_error') THEN 'offline' \
                 WHEN EXISTS (SELECT 1 FROM ai_drafts d \
                     WHERE d.account_id = a.id AND d.status = 'pending' AND d.created_at > ?) \
                     THEN 'processing' \
                 ELSE 'idle' END \
             FROM accounts a \
             LEFT JOIN sync_state ss ON ss.account_id = a.id \
             WHERE a.is_active = 1 \
             ORDER BY a.is_primary DESC, a.created_at ASC, a.id ASC",
        )
        .bind(since)
        .fetch_all(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(rows
            .into_iter()
            .map(|(account_id, status)| AgentStatus { account_id, status })
            .collect())
    }

    /// Record a successful sync's wall-clock time (called by the scheduler).
    pub async fn set_last_synced(&self, id: &str, at: i64) -> AppResult<()> {
        sqlx::query("UPDATE accounts SET last_synced_at = ?, updated_at = ? WHERE id = ?")
            .bind(at)
            .bind(now_unix())
            .bind(id)
            .execute(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(())
    }

    /// Persist the knowledge-depth selection (T016). `None` = "all mail".
    pub async fn set_knowledge_depth(&self, id: &str, months: Option<u32>) -> AppResult<Account> {
        let now = now_unix();
        sqlx::query(
            "UPDATE accounts SET knowledge_depth_months = ?, knowledge_depth_set_at = ?, \
                 updated_at = ? WHERE id = ?",
        )
        .bind(months.map(|m| m as i64))
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        self.get(id).await
    }

    /// Mark an account auth-failed (T018). Delegates to [`SyncStateRepo`], the
    /// single writer of `sync_state` — `is_active` (owned by `accounts`) is
    /// deliberately left unchanged, so the account stays visible (red badge) but
    /// stops polling.
    pub async fn set_auth_failed(&self, id: &str) -> AppResult<()> {
        SyncStateRepo::new(self.db).mark_auth_failed(id).await
    }

    /// Clear the auth-error flag so the scheduler can resume polling (T018 reauth).
    /// Delegates to [`SyncStateRepo`], the single writer of `sync_state`.
    pub async fn clear_auth_error(&self, id: &str) -> AppResult<()> {
        SyncStateRepo::new(self.db).clear_auth_error(id).await
    }

    /// Read the current `last_sync_result` ('auth_error' etc.), if any.
    pub async fn last_sync_result(&self, id: &str) -> AppResult<Option<String>> {
        let row = sqlx::query("SELECT last_sync_result FROM sync_state WHERE account_id = ?")
            .bind(id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(row.and_then(|r| r.get::<Option<String>, _>("last_sync_result")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str, email: &str) -> NewAccount {
        NewAccount {
            id: id.into(),
            email: email.into(),
            display_name: "Work".into(),
            provider: "imap".into(),
            imap_host: Some("imap.example.com".into()),
            imap_port: 993,
            smtp_host: Some("smtp.example.com".into()),
            smtp_port: 587,
            color_token: "slate".into(),
            badge_label: "W".into(),
            role_type: "work".into(),
            role_description: None,
            auth_level: 1,
        }
    }

    async fn db() -> Db {
        let db = Db::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    #[tokio::test]
    async fn create_writes_side_effect_rows_and_first_is_primary() {
        let db = db().await;
        let repo = AccountRepo::new(&db);
        let acct = repo.create(&sample("a1", "a@x.com")).await.unwrap();
        assert!(acct.is_primary, "first account is primary");

        // sync_state + ai_settings rows exist.
        let (sc,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM sync_state WHERE account_id = 'a1'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        let (ai,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM account_ai_settings WHERE account_id = 'a1'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!((sc, ai), (1, 1));
    }

    #[tokio::test]
    async fn duplicate_email_is_db_constraint() {
        let db = db().await;
        let repo = AccountRepo::new(&db);
        repo.create(&sample("a1", "dup@x.com")).await.unwrap();
        let err = repo.create(&sample("a2", "dup@x.com")).await.unwrap_err();
        assert!(matches!(err, AppError::DbConstraint(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn primary_transfer_and_unset_guard() {
        let db = db().await;
        let repo = AccountRepo::new(&db);
        repo.create(&sample("a1", "a@x.com")).await.unwrap();
        repo.create(&sample("a2", "b@x.com")).await.unwrap();

        // Transfer primary to a2.
        let patch = UpdateAccountParams {
            is_primary: Some(true),
            ..Default::default()
        };
        let a2 = repo.update("a2", &patch).await.unwrap();
        assert!(a2.is_primary);
        assert!(!repo.get("a1").await.unwrap().is_primary);

        // Cannot unset the sole primary.
        let unset = UpdateAccountParams {
            is_primary: Some(false),
            ..Default::default()
        };
        assert!(matches!(
            repo.update("a2", &unset).await.unwrap_err(),
            AppError::Forbidden(_)
        ));
    }

    #[tokio::test]
    async fn delete_cascades_children() {
        let db = db().await;
        let repo = AccountRepo::new(&db);
        repo.create(&sample("a1", "a@x.com")).await.unwrap();
        repo.delete("a1").await.unwrap();
        let (sc,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM sync_state WHERE account_id = 'a1'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(sc, 0, "sync_state cascaded away");
    }

    #[tokio::test]
    async fn delete_recovers_from_corrupt_attachments_fts() {
        // Regression for the "can't remove mailbox" bug: a corrupt FTS5 shadow
        // index makes the cascading account delete fail with "database disk image
        // is malformed" (SQLITE_CORRUPT_VTAB, code 267). The corruption can pass
        // `PRAGMA integrity_check` yet still abort the delete-trigger write, so the
        // delete must FORCE an FTS rebuild (not gate it on the integrity check that
        // misses this) and retry. We reproduce the exact real-world profile by
        // corrupting the FTS5 structure record (rowid 1 of the shadow `_data` table).
        let db = db().await;
        let repo = AccountRepo::new(&db);
        repo.create(&sample("a1", "a@x.com")).await.unwrap();

        // A mail with an indexed attachment, so `attachments_fts` has content and
        // the account-delete cascade fires `attachments_fts_after_delete`.
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, from_email, to_addrs, date_sent, \
             date_received, body_text, created_at, updated_at) \
             VALUES ('m1','a1','<1@x>','s@x.y','[]',1,1,'see attached',0,0)",
        )
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO attachments (id, mail_id, account_id, filename, content_type, \
             size_bytes, created_at) \
             VALUES ('at1','m1','a1','invoice.pdf','application/pdf',1024,0)",
        )
        .execute(db.pool())
        .await
        .unwrap();
        // Fire the index trigger so the attachment lands in `attachments_fts`.
        sqlx::query(
            "UPDATE attachments SET extraction_status='indexed', \
             extracted_text='quarterly budget invoice totals enclosed' WHERE id='at1'",
        )
        .execute(db.pool())
        .await
        .unwrap();

        // Corrupt the FTS5 structure record. This mirrors the real failure: a write
        // that fires the FTS delete trigger now hits a malformed shadow page.
        sqlx::query(
            "UPDATE attachments_fts_data SET block = randomblob(length(block)) WHERE id = 1",
        )
        .execute(db.pool())
        .await
        .unwrap();
        // The cheap check the old self-heal relied on stays clean — which is exactly
        // why removing the mailbox was permanently blocked before the fix.
        let (chk,): (String,) = sqlx::query_as("PRAGMA integrity_check")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(chk, "ok", "integrity_check is blind to this FTS corruption");

        // The fix: the delete forces a full FTS rebuild and then succeeds.
        repo.delete("a1")
            .await
            .expect("delete recovers from corrupt attachments_fts");
        assert!(matches!(repo.get("a1").await, Err(AppError::NotFound)));
    }

    #[tokio::test]
    async fn knowledge_depth_roundtrip_and_auth_failed() {
        let db = db().await;
        let repo = AccountRepo::new(&db);
        repo.create(&sample("a1", "a@x.com")).await.unwrap();

        let a = repo.set_knowledge_depth("a1", Some(12)).await.unwrap();
        assert_eq!(a.knowledge_depth_months, Some(12));
        let a = repo.set_knowledge_depth("a1", None).await.unwrap();
        assert_eq!(a.knowledge_depth_months, None);

        repo.set_auth_failed("a1").await.unwrap();
        assert_eq!(
            repo.last_sync_result("a1").await.unwrap().as_deref(),
            Some("auth_error")
        );
        // is_active unchanged.
        assert!(repo.get("a1").await.unwrap().is_active);
        repo.clear_auth_error("a1").await.unwrap();
        assert_eq!(repo.last_sync_result("a1").await.unwrap(), None);
    }

    #[tokio::test]
    async fn set_primary_promotes_and_demotes_atomically() {
        let db = db().await;
        let repo = AccountRepo::new(&db);
        repo.create(&sample("a1", "a@x.com")).await.unwrap();
        repo.create(&sample("a2", "b@x.com")).await.unwrap();
        assert!(repo.get("a1").await.unwrap().is_primary);

        let a2 = repo.set_primary("a2").await.unwrap();
        assert!(a2.is_primary);
        assert!(!repo.get("a1").await.unwrap().is_primary);

        // Exactly one primary remains after the swap.
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM accounts WHERE is_primary = 1")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn set_primary_unknown_account_is_not_found() {
        let db = db().await;
        let repo = AccountRepo::new(&db);
        repo.create(&sample("a1", "a@x.com")).await.unwrap();
        assert!(matches!(
            repo.set_primary("missing").await.unwrap_err(),
            AppError::NotFound
        ));
    }

    #[tokio::test]
    async fn heal_primary_fixes_zero_and_multiple() {
        let db = db().await;
        let repo = AccountRepo::new(&db);
        repo.create(&sample("a1", "a@x.com")).await.unwrap();
        repo.create(&sample("a2", "b@x.com")).await.unwrap();

        // Corrupt: zero primaries.
        sqlx::query("UPDATE accounts SET is_primary = 0")
            .execute(db.pool())
            .await
            .unwrap();
        let was = repo.heal_primary().await.unwrap();
        assert_eq!(was, 0, "reported zero primaries before healing");
        // Earliest-created active account (a1) is promoted.
        assert!(repo.get("a1").await.unwrap().is_primary);
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM accounts WHERE is_primary = 1")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(n, 1);

        // Corrupt: two primaries.
        sqlx::query("UPDATE accounts SET is_primary = 1")
            .execute(db.pool())
            .await
            .unwrap();
        let was = repo.heal_primary().await.unwrap();
        assert_eq!(was, 2, "reported two primaries before healing");
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM accounts WHERE is_primary = 1")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(n, 1, "healed back to a single primary");
    }

    #[tokio::test]
    async fn heal_primary_noop_when_already_single() {
        let db = db().await;
        let repo = AccountRepo::new(&db);
        repo.create(&sample("a1", "a@x.com")).await.unwrap();
        // Healthy db: exactly one primary → reported as one, unchanged.
        assert_eq!(repo.heal_primary().await.unwrap(), 1);
        assert!(repo.get("a1").await.unwrap().is_primary);
    }

    #[tokio::test]
    async fn agent_statuses_derives_presence() {
        let db = db().await;
        let repo = AccountRepo::new(&db);
        repo.create(&sample("a1", "a@x.com")).await.unwrap();

        // Default: no sync failure, no recent drafts → idle.
        let statuses = repo.agent_statuses().await.unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].account_id, "a1");
        assert_eq!(statuses[0].status, "idle");

        // Offline: auth error on sync_state.
        repo.set_auth_failed("a1").await.unwrap();
        assert_eq!(repo.agent_statuses().await.unwrap()[0].status, "offline");

        // Processing: clear the error, add a recent pending draft.
        repo.clear_auth_error("a1").await.unwrap();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, from_email, to_addrs, date_sent, \
                 date_received, created_at, updated_at) \
             VALUES ('m1','a1','<m1>','x@y.com','[]', ?, ?, ?, ?)",
        )
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO ai_drafts (id, trigger_mail_id, account_id, to_addr, subject, \
                 body_original, body_current, trigger_mode, ai_model, status, created_at, updated_at) \
             VALUES ('d1','m1','a1','{\"email\":\"x@y.com\"}','Re','b','b','E2_semi','mock','pending', ?, ?)",
        )
        .bind(now)
        .bind(now)
        .execute(db.pool())
        .await
        .unwrap();
        assert_eq!(repo.agent_statuses().await.unwrap()[0].status, "processing");
    }
}
