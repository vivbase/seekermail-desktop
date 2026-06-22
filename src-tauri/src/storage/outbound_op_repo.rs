//! `OutboundOpRepo` — the durable write-back queue (Phase 2, migration 019).
//!
//! A local read/star toggle records an op here; the drain worker
//! ([`crate::imap::outbound`]) applies it to the server via `UID STORE` and marks
//! it done. Durable so actions survive a restart or offline window; `STORE
//! +/-FLAGS` is idempotent, so re-applying a claimed-but-uncommitted op is
//! harmless. Repeated toggles on one message coalesce to the latest intent (see
//! [`OutboundOpRepo::enqueue`]).

use super::{map_sqlx_err, Db};
use crate::error::AppResult;
use crate::util::{new_uuid, now_unix};

/// Max attempts before an op is parked `failed`, so a permanently broken op
/// (e.g. its UID vanished after a server-side move) can't retry forever.
pub const OUTBOUND_OP_MAX_ATTEMPTS: i64 = 5;

/// The kind of write-back. Each maps 1:1 to a `UID STORE` of one system flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundOpKind {
    // Flag ops → `UID STORE` one system flag.
    MarkSeen,
    MarkUnseen,
    Flag,
    Unflag,
    // Move ops → `UID MOVE` to a role folder: archive, delete (→ Trash),
    // mark-spam (→ Junk), restore (→ Inbox, the inverse of the three above).
    Archive,
    Trash,
    MarkSpam,
    Restore,
}

impl OutboundOpKind {
    pub fn as_str(self) -> &'static str {
        match self {
            OutboundOpKind::MarkSeen => "mark_seen",
            OutboundOpKind::MarkUnseen => "mark_unseen",
            OutboundOpKind::Flag => "flag",
            OutboundOpKind::Unflag => "unflag",
            OutboundOpKind::Archive => "archive",
            OutboundOpKind::Trash => "trash",
            OutboundOpKind::MarkSpam => "mark_spam",
            OutboundOpKind::Restore => "restore",
        }
    }

    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "mark_seen" => Some(OutboundOpKind::MarkSeen),
            "mark_unseen" => Some(OutboundOpKind::MarkUnseen),
            "flag" => Some(OutboundOpKind::Flag),
            "unflag" => Some(OutboundOpKind::Unflag),
            "archive" => Some(OutboundOpKind::Archive),
            "trash" => Some(OutboundOpKind::Trash),
            "mark_spam" => Some(OutboundOpKind::MarkSpam),
            "restore" => Some(OutboundOpKind::Restore),
            _ => None,
        }
    }

    /// The op types in the same family, used to coalesce so the queue keeps only
    /// the latest intent for a message. Flags coalesce within their on/off pair; a
    /// message can only live in one place, so any relocation supersedes a pending
    /// one.
    fn family(self) -> &'static [&'static str] {
        match self {
            OutboundOpKind::MarkSeen | OutboundOpKind::MarkUnseen => &["mark_seen", "mark_unseen"],
            OutboundOpKind::Flag | OutboundOpKind::Unflag => &["flag", "unflag"],
            // All relocations share one family: a message lives in exactly one place,
            // so any new move (including a restore-to-Inbox) supersedes a pending one.
            // This is what makes an undo before the Trash move drains cancel it.
            OutboundOpKind::Archive
            | OutboundOpKind::Trash
            | OutboundOpKind::MarkSpam
            | OutboundOpKind::Restore => &["archive", "trash", "mark_spam", "restore"],
        }
    }
}

/// One claimed write-back, ready to apply to the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundOp {
    pub id: String,
    pub account_id: String,
    pub folder: String,
    pub imap_uid: i64,
    pub kind: OutboundOpKind,
    pub attempts: i64,
}

#[derive(sqlx::FromRow)]
struct OutboundOpRow {
    id: String,
    account_id: String,
    folder: String,
    imap_uid: i64,
    op_type: String,
    attempts: i64,
}

#[derive(Clone)]
pub struct OutboundOpRepo<'a> {
    db: &'a Db,
}

impl<'a> OutboundOpRepo<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Enqueue a write-back, first dropping any pending op in the same flag family
    /// for the same message (the latest toggle wins). Returns the new op id.
    pub async fn enqueue(
        &self,
        account_id: &str,
        folder: &str,
        imap_uid: i64,
        kind: OutboundOpKind,
    ) -> AppResult<String> {
        // Coalesce: drop pending ops in the same family for this message (latest
        // intent wins). Built dynamically because families vary in length.
        let mut del = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "DELETE FROM outbound_ops WHERE status = 'pending' AND account_id = ",
        );
        del.push_bind(account_id)
            .push(" AND folder = ")
            .push_bind(folder)
            .push(" AND imap_uid = ")
            .push_bind(imap_uid)
            .push(" AND op_type IN (");
        {
            let mut sep = del.separated(", ");
            for t in kind.family() {
                sep.push_bind(*t);
            }
        }
        del.push(")");
        del.build()
            .execute(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;

        let id = new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO outbound_ops (id, account_id, folder, imap_uid, op_type, status, \
                 attempts, created_at, updated_at) VALUES (?, ?, ?, ?, ?, 'pending', 0, ?, ?)",
        )
        .bind(&id)
        .bind(account_id)
        .bind(folder)
        .bind(imap_uid)
        .bind(kind.as_str())
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(id)
    }

    /// Claim up to `limit` pending ops for an account, oldest first.
    pub async fn claim_pending(&self, account_id: &str, limit: i64) -> AppResult<Vec<OutboundOp>> {
        let rows: Vec<OutboundOpRow> = sqlx::query_as(
            "SELECT id, account_id, folder, imap_uid, op_type, attempts FROM outbound_ops \
             WHERE account_id = ? AND status = 'pending' ORDER BY created_at ASC LIMIT ?",
        )
        .bind(account_id)
        .bind(limit)
        .fetch_all(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(rows
            .into_iter()
            .filter_map(|r| {
                OutboundOpKind::from_wire(&r.op_type).map(|kind| OutboundOp {
                    id: r.id,
                    account_id: r.account_id,
                    folder: r.folder,
                    imap_uid: r.imap_uid,
                    kind,
                    attempts: r.attempts,
                })
            })
            .collect())
    }

    /// `(folder, imap_uid)` of every pending op for an account — so inbound
    /// reconciliation can avoid clobbering a local change that hasn't been written
    /// back to the server yet.
    pub async fn pending_targets(&self, account_id: &str) -> AppResult<Vec<(String, i64)>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT folder, imap_uid FROM outbound_ops WHERE account_id = ? AND status = 'pending'",
        )
        .bind(account_id)
        .fetch_all(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(rows)
    }

    /// Account ids that currently have pending ops (drives the drain scheduler).
    pub async fn accounts_with_pending(&self) -> AppResult<Vec<String>> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT DISTINCT account_id FROM outbound_ops WHERE status = 'pending'")
                .fetch_all(self.db.pool())
                .await
                .map_err(map_sqlx_err)?;
        Ok(rows.into_iter().map(|(a,)| a).collect())
    }

    pub async fn mark_done(&self, id: &str) -> AppResult<()> {
        sqlx::query("UPDATE outbound_ops SET status = 'done', updated_at = ? WHERE id = ?")
            .bind(now_unix())
            .bind(id)
            .execute(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(())
    }

    /// Record a failed attempt: bump `attempts`, store the error, and park the op
    /// `failed` once it has burned [`OUTBOUND_OP_MAX_ATTEMPTS`] tries (otherwise it
    /// stays `pending` for the next drain).
    pub async fn mark_failed(&self, id: &str, attempts: i64, err: &str) -> AppResult<()> {
        let next = attempts + 1;
        let status = if next >= OUTBOUND_OP_MAX_ATTEMPTS {
            "failed"
        } else {
            "pending"
        };
        sqlx::query(
            "UPDATE outbound_ops SET status = ?, attempts = ?, last_error = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(status)
        .bind(next)
        .bind(err)
        .bind(now_unix())
        .bind(id)
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

    /// A read toggle storm coalesces: only the latest intent stays pending.
    #[tokio::test]
    async fn enqueue_coalesces_same_family_toggles() {
        let db = db_with_account().await;
        let repo = OutboundOpRepo::new(&db);

        repo.enqueue("a1", "INBOX", 7, OutboundOpKind::MarkSeen)
            .await
            .unwrap();
        repo.enqueue("a1", "INBOX", 7, OutboundOpKind::MarkUnseen)
            .await
            .unwrap();
        repo.enqueue("a1", "INBOX", 7, OutboundOpKind::MarkSeen)
            .await
            .unwrap();
        // A star op is a different family — it survives alongside the seen op.
        repo.enqueue("a1", "INBOX", 7, OutboundOpKind::Flag)
            .await
            .unwrap();

        let pending = repo.claim_pending("a1", 50).await.unwrap();
        assert_eq!(pending.len(), 2, "one coalesced seen op + one flag op");
        assert!(pending.iter().any(|o| o.kind == OutboundOpKind::MarkSeen));
        assert!(pending.iter().any(|o| o.kind == OutboundOpKind::Flag));
        assert!(!pending.iter().any(|o| o.kind == OutboundOpKind::MarkUnseen));
    }

    /// done removes an op from the pending set; failed retries until the cap.
    #[tokio::test]
    async fn done_and_failed_lifecycle() {
        let db = db_with_account().await;
        let repo = OutboundOpRepo::new(&db);

        let id = repo
            .enqueue("a1", "INBOX", 9, OutboundOpKind::Flag)
            .await
            .unwrap();
        assert_eq!(repo.accounts_with_pending().await.unwrap(), vec!["a1"]);

        // Fail it up to the cap → it parks `failed` and leaves the pending set.
        for attempt in 0..OUTBOUND_OP_MAX_ATTEMPTS {
            repo.mark_failed(&id, attempt, "imap down").await.unwrap();
        }
        assert!(
            repo.claim_pending("a1", 50).await.unwrap().is_empty(),
            "op parked failed after the attempt cap"
        );

        // A fresh op that succeeds is marked done and drops out of pending.
        let id2 = repo
            .enqueue("a1", "INBOX", 10, OutboundOpKind::MarkSeen)
            .await
            .unwrap();
        repo.mark_done(&id2).await.unwrap();
        assert!(repo.accounts_with_pending().await.unwrap().is_empty());
    }

    /// A restore issued before the Trash move drained supersedes it: relocations
    /// share one family, so only the latest intent survives (an in-place undo).
    #[tokio::test]
    async fn restore_coalesces_a_pending_relocation() {
        let db = db_with_account().await;
        let repo = OutboundOpRepo::new(&db);

        repo.enqueue("a1", "INBOX", 7, OutboundOpKind::Trash)
            .await
            .unwrap();
        repo.enqueue("a1", "INBOX", 7, OutboundOpKind::Restore)
            .await
            .unwrap();

        let pending = repo.claim_pending("a1", 50).await.unwrap();
        assert_eq!(
            pending.len(),
            1,
            "the relocation family coalesces to one op"
        );
        assert_eq!(pending[0].kind, OutboundOpKind::Restore);
    }
}
