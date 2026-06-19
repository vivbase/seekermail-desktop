//! `MailRepo` — the parse→persist heart (T023).
//!
//! `upsert_batch` writes a whole fetch batch in ONE transaction: thread
//! resolution, de-duplicated mail insert, attachment metadata, and thread
//! aggregate bumps. FTS5 stays in sync automatically via the triggers in
//! `001_init.sql`, so there's no FTS code here.

use super::{map_sqlx_err, Db};
use crate::error::AppResult;
use crate::imap::thread;
use crate::types::{
    ListMailsParams, ListThreadsParams, MailDetail, MailSummary, PageResult, ParsedMail, Recipient,
    Thread, UpsertStats,
};
use crate::util::{new_uuid, now_unix, truncate_chars};

/// One persisted mail, returned so the worker can emit `mail:new`.
#[derive(Debug, Clone)]
pub struct Inserted {
    pub summary: MailSummary,
}

#[derive(Clone)]
pub struct MailRepo<'a> {
    db: &'a Db,
}

impl<'a> MailRepo<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Persist a batch of parsed mails. Returns counts + the rows actually
    /// inserted (for `mail:new`). Duplicates (same `account_id, message_id`) are
    /// counted and skipped.
    pub async fn upsert_batch(
        &self,
        mails: &[ParsedMail],
    ) -> AppResult<(UpsertStats, Vec<Inserted>)> {
        let now = now_unix();
        let mut stats = UpsertStats::default();
        let mut inserted: Vec<Inserted> = Vec::new();
        let mut tx = self.db.pool().begin().await.map_err(map_sqlx_err)?;

        for mail in mails {
            // Dedup by (account_id, message_id).
            let dup = sqlx::query("SELECT 1 FROM mails WHERE account_id = ? AND message_id = ?")
                .bind(&mail.account_id)
                .bind(&mail.message_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(map_sqlx_err)?;
            if dup.is_some() {
                stats.skipped_duplicate += 1;
                continue;
            }

            let thread_id = thread::resolve(&mut tx, mail, now).await?;
            let mail_id = new_uuid();
            let snippet = mail.snippet.clone().unwrap_or_default();

            sqlx::query(
                "INSERT INTO mails (id, account_id, thread_id, message_id, in_reply_to, \"references\", \
                     subject, from_name, from_email, to_addrs, cc_addrs, bcc_addrs, reply_to, \
                     date_sent, date_received, body_text, body_html, snippet, is_read, folder, \
                     imap_uid, has_attachments, tracker_blocked, tracker_count, embedding_status, \
                     created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?, ?, ?, ?, \
                     'pending', ?, ?)",
            )
            .bind(&mail_id)
            .bind(&mail.account_id)
            .bind(&thread_id)
            .bind(&mail.message_id)
            .bind(&mail.in_reply_to)
            .bind(&mail.references)
            .bind(&mail.subject)
            .bind(&mail.from_name)
            .bind(&mail.from_email)
            .bind(&mail.to_addrs)
            .bind(&mail.cc_addrs)
            .bind(&mail.bcc_addrs)
            .bind(&mail.reply_to)
            .bind(mail.date_sent)
            .bind(mail.date_received)
            .bind(&mail.body_text)
            .bind(&mail.body_html)
            .bind(truncate_chars(&snippet, 200))
            .bind(&mail.folder)
            .bind(mail.imap_uid)
            .bind(mail.has_attachments as i64)
            .bind((mail.tracker_count > 0) as i64)
            .bind(mail.tracker_count as i64)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_err)?;

            // Attachment metadata (bytes downloaded later by T025).
            for att in &mail.attachments {
                sqlx::query(
                    "INSERT INTO attachments (id, mail_id, content_id, filename, content_type, \
                         size_bytes, is_inline, part_index, downloaded, created_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, ?)",
                )
                .bind(new_uuid())
                .bind(&mail_id)
                .bind(&att.content_id)
                .bind(&att.filename)
                .bind(&att.content_type)
                .bind(att.size_bytes as i64)
                .bind(att.is_inline as i64)
                .bind(att.part_index as i64)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx_err)?;
            }

            // Thread aggregates (RHS sees pre-update values in SQLite).
            sqlx::query(
                "UPDATE threads SET mail_count = mail_count + 1, \
                     unread_count = unread_count + 1, \
                     has_attachments = CASE WHEN ? = 1 THEN 1 ELSE has_attachments END, \
                     snippet = CASE WHEN ? >= latest_date THEN ? ELSE snippet END, \
                     latest_date = MAX(latest_date, ?), updated_at = ? WHERE id = ?",
            )
            .bind(mail.has_attachments as i64)
            .bind(mail.date_sent)
            .bind(truncate_chars(&snippet, 160))
            .bind(mail.date_sent)
            .bind(now)
            .bind(&thread_id)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_err)?;

            stats.inserted += 1;
            inserted.push(Inserted {
                summary: MailSummary {
                    id: mail_id,
                    account_id: mail.account_id.clone(),
                    thread_id: Some(thread_id),
                    subject: mail.subject.clone(),
                    from_name: mail.from_name.clone(),
                    from_email: mail.from_email.clone(),
                    snippet: Some(truncate_chars(&snippet, 200)),
                    date_sent: mail.date_sent,
                    is_read: false,
                    has_attachments: mail.has_attachments,
                },
            });
        }

        tx.commit().await.map_err(map_sqlx_err)?;
        Ok((stats, inserted))
    }

    /// Mark a mail read/unread (drives `mail:updated`).
    pub async fn set_read(&self, mail_id: &str, read: bool) -> AppResult<()> {
        sqlx::query("UPDATE mails SET is_read = ?, updated_at = ? WHERE id = ?")
            .bind(read as i64)
            .bind(now_unix())
            .bind(mail_id)
            .execute(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(())
    }

    /// IMAP fetch context for an attachment download (T025): the mail's account,
    /// UID, and sent year/month for the on-disk path.
    pub async fn fetch_context(&self, mail_id: &str) -> AppResult<MailFetchContext> {
        use sqlx::Row;
        let row = sqlx::query("SELECT account_id, imap_uid, date_sent FROM mails WHERE id = ?")
            .bind(mail_id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(map_sqlx_err)?
            .ok_or(crate::error::AppError::NotFound)?;
        let date_sent: i64 = row.get("date_sent");
        let (year, month) = year_month(date_sent);
        Ok(MailFetchContext {
            account_id: row.get("account_id"),
            imap_uid: row.get::<Option<i64>, _>("imap_uid"),
            year,
            month,
        })
    }

    /// Sender + tracker fields for one mail (T029 `get_tracker_info`).
    pub async fn tracker_row(&self, mail_id: &str) -> AppResult<(String, bool, u32)> {
        use sqlx::Row;
        let row = sqlx::query(
            "SELECT from_email, tracker_blocked, tracker_count FROM mails WHERE id = ?",
        )
        .bind(mail_id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(map_sqlx_err)?
        .ok_or(crate::error::AppError::NotFound)?;
        Ok((
            row.get::<String, _>("from_email"),
            row.get::<i64, _>("tracker_blocked") != 0,
            row.get::<i64, _>("tracker_count").max(0) as u32,
        ))
    }

    /// Count mails for an account (test/diagnostics).
    pub async fn count_for_account(&self, account_id: &str) -> AppResult<i64> {
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM mails WHERE account_id = ?")
            .bind(account_id)
            .fetch_one(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(n)
    }

    // ── B3 embedding lifecycle (T031) ────────────────────────────────────────

    /// Batch-transition `embedding_status` for the given mails (`pending → indexed
    /// / skipped / error`). `model`/`embedded_at` are only written when `Some`
    /// (so a `skipped`/`error` transition leaves them untouched). Idempotent —
    /// re-running over already-`indexed` rows just re-stamps them.
    pub async fn update_embedding_status(
        &self,
        ids: &[String],
        status: &str,
        model: Option<&str>,
        embedded_at: Option<i64>,
    ) -> AppResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let placeholders = vec!["?"; ids.len()].join(",");
        let sql = format!(
            "UPDATE mails SET embedding_status = ?, \
                 embedding_model = COALESCE(?, embedding_model), \
                 embedded_at = COALESCE(?, embedded_at), \
                 updated_at = ? WHERE id IN ({placeholders})"
        );
        let mut q = sqlx::query(&sql)
            .bind(status)
            .bind(model)
            .bind(embedded_at)
            .bind(now_unix());
        for id in ids {
            q = q.bind(id);
        }
        q.execute(self.db.pool()).await.map_err(map_sqlx_err)?;
        Ok(())
    }

    /// Pull up to `limit` mails still awaiting embedding (the "catch-up" path that
    /// recovers anything the bounded channel dropped or that predates this run).
    pub async fn next_embedding_batch(
        &self,
        limit: i64,
    ) -> AppResult<Vec<crate::embedding::queue::EmbedJob>> {
        use sqlx::Row;
        let rows = sqlx::query(
            "SELECT id, account_id, from_email, date_sent, subject, \
                 COALESCE(snippet, '') AS snippet, COALESCE(body_text, '') AS body_text \
             FROM mails WHERE embedding_status = 'pending' ORDER BY date_sent DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(rows
            .iter()
            .map(|r| crate::embedding::queue::EmbedJob {
                mail_id: r.get("id"),
                account_id: r.get("account_id"),
                from_email: r.get("from_email"),
                date_sent: r.get("date_sent"),
                subject: r.get("subject"),
                snippet: r.get("snippet"),
                body_text: r.get("body_text"),
                retry: 0,
            })
            .collect())
    }

    /// How many mails are still `embedding_status='pending'` (drives `gte:progress`).
    pub async fn count_pending_embeddings(&self) -> AppResult<i64> {
        let (n,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM mails WHERE embedding_status = 'pending'")
                .fetch_one(self.db.pool())
                .await
                .map_err(map_sqlx_err)?;
        Ok(n)
    }

    // ── Read backend for the list + reading views (G2/G3) ─────────────────────

    /// Paginated thread list (folded L0 stream). Newest-first by `latest_date`.
    pub async fn list_threads(&self, p: &ListThreadsParams) -> AppResult<PageResult<Thread>> {
        use sqlx::{QueryBuilder, Row, Sqlite};

        let mut cqb = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) AS cnt FROM threads WHERE 1=1");
        push_thread_filters(&mut cqb, p);
        let total: i64 = cqb
            .build()
            .fetch_one(self.db.pool())
            .await
            .map_err(map_sqlx_err)?
            .get("cnt");

        let mut qb = QueryBuilder::<Sqlite>::new(
            "SELECT id, account_id, subject, participants, mail_count, unread_count, \
                 has_attachments, latest_date, snippet, is_archived, is_starred \
             FROM threads WHERE 1=1",
        );
        push_thread_filters(&mut qb, p);
        qb.push(" ORDER BY latest_date DESC LIMIT ")
            .push_bind(p.limit.max(0))
            .push(" OFFSET ")
            .push_bind(p.offset.max(0));
        let rows = qb
            .build()
            .fetch_all(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;

        let items = rows
            .iter()
            .map(|r| Thread {
                id: r.get("id"),
                account_id: r.get("account_id"),
                subject: r.get("subject"),
                participants: parse_str_array(r.get::<String, _>("participants").as_str()),
                mail_count: r.get::<i64, _>("mail_count").max(0) as u32,
                unread_count: r.get::<i64, _>("unread_count").max(0) as u32,
                has_attachments: r.get::<i64, _>("has_attachments") != 0,
                latest_date: r.get("latest_date"),
                snippet: r.get("snippet"),
                is_archived: r.get::<i64, _>("is_archived") != 0,
                is_starred: r.get::<i64, _>("is_starred") != 0,
            })
            .collect();

        Ok(PageResult {
            items,
            total: total.max(0) as u32,
            offset: p.offset.max(0) as u32,
        })
    }

    /// Paginated flat mail list. Excludes soft-deleted rows; newest-first by
    /// `date_received`.
    pub async fn list_mails(&self, p: &ListMailsParams) -> AppResult<PageResult<MailSummary>> {
        use sqlx::{QueryBuilder, Row, Sqlite};

        let mut cqb =
            QueryBuilder::<Sqlite>::new("SELECT COUNT(*) AS cnt FROM mails WHERE is_deleted = 0");
        push_mail_filters(&mut cqb, p);
        let total: i64 = cqb
            .build()
            .fetch_one(self.db.pool())
            .await
            .map_err(map_sqlx_err)?
            .get("cnt");

        let mut qb = QueryBuilder::<Sqlite>::new(
            "SELECT id, account_id, thread_id, subject, from_name, from_email, snippet, \
                 date_sent, is_read, has_attachments \
             FROM mails WHERE is_deleted = 0",
        );
        push_mail_filters(&mut qb, p);
        qb.push(" ORDER BY date_received DESC LIMIT ")
            .push_bind(p.limit.max(0))
            .push(" OFFSET ")
            .push_bind(p.offset.max(0));
        let rows = qb
            .build()
            .fetch_all(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;

        let items = rows
            .iter()
            .map(|r| MailSummary {
                id: r.get("id"),
                account_id: r.get("account_id"),
                thread_id: r.get("thread_id"),
                subject: r.get("subject"),
                from_name: r.get("from_name"),
                from_email: r.get("from_email"),
                snippet: r.get("snippet"),
                date_sent: r.get("date_sent"),
                is_read: r.get::<i64, _>("is_read") != 0,
                has_attachments: r.get::<i64, _>("has_attachments") != 0,
            })
            .collect();

        Ok(PageResult {
            items,
            total: total.max(0) as u32,
            offset: p.offset.max(0) as u32,
        })
    }

    /// Full mail for the reading view. `to`/`cc` decoded from stored JSON.
    pub async fn get_mail(&self, mail_id: &str) -> AppResult<MailDetail> {
        use sqlx::Row;
        let row = sqlx::query(
            "SELECT id, account_id, thread_id, subject, from_name, from_email, to_addrs, \
                 cc_addrs, date_sent, body_html, body_text, is_read, is_starred, is_archived, \
                 has_attachments, folder FROM mails WHERE id = ?",
        )
        .bind(mail_id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(map_sqlx_err)?
        .ok_or(crate::error::AppError::NotFound)?;

        Ok(MailDetail {
            id: row.get("id"),
            account_id: row.get("account_id"),
            thread_id: row.get("thread_id"),
            subject: row.get("subject"),
            from_name: row.get("from_name"),
            from_email: row.get("from_email"),
            to: parse_recipients(row.get::<Option<String>, _>("to_addrs").as_deref()),
            cc: parse_recipients(row.get::<Option<String>, _>("cc_addrs").as_deref()),
            date_sent: row.get("date_sent"),
            body_html: row.get("body_html"),
            body_text: row.get("body_text"),
            is_read: row.get::<i64, _>("is_read") != 0,
            is_starred: row.get::<i64, _>("is_starred") != 0,
            is_archived: row.get::<i64, _>("is_archived") != 0,
            has_attachments: row.get::<i64, _>("has_attachments") != 0,
            folder: row.get("folder"),
        })
    }

    /// Star / unstar a mail.
    pub async fn set_starred(&self, mail_id: &str, starred: bool) -> AppResult<()> {
        sqlx::query("UPDATE mails SET is_starred = ?, updated_at = ? WHERE id = ?")
            .bind(starred as i64)
            .bind(now_unix())
            .bind(mail_id)
            .execute(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(())
    }

    /// Archive / unarchive a mail.
    pub async fn set_archived(&self, mail_id: &str, archived: bool) -> AppResult<()> {
        sqlx::query("UPDATE mails SET is_archived = ?, updated_at = ? WHERE id = ?")
            .bind(archived as i64)
            .bind(now_unix())
            .bind(mail_id)
            .execute(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(())
    }

    /// Soft-delete / restore a mail (Trash; `is_deleted`).
    pub async fn set_deleted(&self, mail_id: &str, deleted: bool) -> AppResult<()> {
        sqlx::query("UPDATE mails SET is_deleted = ?, updated_at = ? WHERE id = ?")
            .bind(deleted as i64)
            .bind(now_unix())
            .bind(mail_id)
            .execute(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(())
    }
}

/// Append the optional WHERE clauses for `list_threads`.
fn push_thread_filters(qb: &mut sqlx::QueryBuilder<'_, sqlx::Sqlite>, p: &ListThreadsParams) {
    if let Some(acc) = p.account_id.clone() {
        qb.push(" AND account_id = ").push_bind(acc);
    }
    if let Some(archived) = p.is_archived {
        qb.push(" AND is_archived = ").push_bind(archived as i64);
    }
    if p.has_unread == Some(true) {
        qb.push(" AND unread_count > 0");
    }
}

/// Append the optional WHERE clauses for `list_mails`.
fn push_mail_filters(qb: &mut sqlx::QueryBuilder<'_, sqlx::Sqlite>, p: &ListMailsParams) {
    if let Some(acc) = p.account_id.clone() {
        qb.push(" AND account_id = ").push_bind(acc);
    }
    if let Some(thread) = p.thread_id.clone() {
        qb.push(" AND thread_id = ").push_bind(thread);
    }
    if let Some(folder) = p.folder.clone() {
        qb.push(" AND folder = ").push_bind(folder);
    }
    if p.is_unread == Some(true) {
        qb.push(" AND is_read = 0");
    }
    if let Some(from) = p.date_from {
        qb.push(" AND date_sent >= ").push_bind(from);
    }
    if let Some(to) = p.date_to {
        qb.push(" AND date_sent <= ").push_bind(to);
    }
}

/// Decode a JSON array of email strings (or `{name,email}` objects) into emails.
fn parse_str_array(s: &str) -> Vec<String> {
    serde_json::from_str::<Vec<serde_json::Value>>(s)
        .map(|arr| {
            arr.into_iter()
                .filter_map(|v| match v {
                    serde_json::Value::String(s) => Some(s),
                    serde_json::Value::Object(o) => o
                        .get("email")
                        .and_then(|e| e.as_str())
                        .map(|e| e.to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Decode a stored JSON address array (`[{name,email}]` or `["email"]`).
fn parse_recipients(s: Option<&str>) -> Vec<Recipient> {
    let Some(s) = s else { return Vec::new() };
    serde_json::from_str::<Vec<serde_json::Value>>(s)
        .map(|arr| {
            arr.into_iter()
                .filter_map(|v| match v {
                    serde_json::Value::String(email) => Some(Recipient { name: None, email }),
                    serde_json::Value::Object(o) => {
                        let email = o.get("email").and_then(|e| e.as_str())?.to_string();
                        let name = o
                            .get("name")
                            .and_then(|n| n.as_str())
                            .filter(|n| !n.is_empty())
                            .map(|n| n.to_string());
                        Some(Recipient { name, email })
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Context needed to fetch + store an attachment's bytes.
#[derive(Debug, Clone)]
pub struct MailFetchContext {
    pub account_id: String,
    pub imap_uid: Option<i64>,
    pub year: u32,
    pub month: u32,
}

/// Unix seconds → (year, month) in UTC.
fn year_month(ts: i64) -> (u32, u32) {
    use chrono::{Datelike, TimeZone, Utc};
    let dt = Utc.timestamp_opt(ts, 0).single().unwrap_or_else(Utc::now);
    (dt.year() as u32, dt.month())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::account_repo::{AccountRepo, NewAccount};

    fn parsed(account: &str, msgid: &str, subject: &str, irt: Option<&str>) -> ParsedMail {
        ParsedMail {
            account_id: account.into(),
            folder: "INBOX".into(),
            imap_uid: Some(1),
            message_id: msgid.into(),
            in_reply_to: irt.map(|s| s.to_string()),
            references: None,
            subject: subject.into(),
            from_name: Some("Alice".into()),
            from_email: "alice@x.com".into(),
            to_addrs: "[]".into(),
            cc_addrs: "[]".into(),
            bcc_addrs: "[]".into(),
            reply_to: None,
            date_sent: 1_700_000_000,
            date_received: 1_700_000_000,
            body_text: Some("hello body".into()),
            body_html: Some("<p>hello body</p>".into()),
            snippet: Some("hello body".into()),
            has_attachments: false,
            tracker_count: 0,
            attachments: vec![],
        }
    }

    async fn db_with_account() -> Db {
        let db = Db::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        AccountRepo::new(&db)
            .create(&NewAccount {
                id: "acc".into(),
                email: "me@x.com".into(),
                display_name: "Me".into(),
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
    async fn single_mail_creates_thread_and_is_searchable() {
        let db = db_with_account().await;
        let repo = MailRepo::new(&db);
        let (stats, inserted) = repo
            .upsert_batch(&[parsed("acc", "<m1@x>", "Hello", None)])
            .await
            .unwrap();
        assert_eq!(stats.inserted, 1);
        assert_eq!(inserted.len(), 1);

        // FTS trigger populated mails_fts.
        let (hits,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM mails_fts WHERE mails_fts MATCH 'hello'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(hits, 1);
    }

    #[tokio::test]
    async fn reply_chain_shares_thread() {
        let db = db_with_account().await;
        let repo = MailRepo::new(&db);
        repo.upsert_batch(&[parsed("acc", "<m1@x>", "Hello", None)])
            .await
            .unwrap();
        repo.upsert_batch(&[parsed("acc", "<m2@x>", "Re: Hello", Some("<m1@x>"))])
            .await
            .unwrap();
        repo.upsert_batch(&[parsed("acc", "<m3@x>", "Re: Hello", Some("<m2@x>"))])
            .await
            .unwrap();

        let (threads,): (i64,) = sqlx::query_as("SELECT count(*) FROM threads")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(threads, 1, "all three share one thread");
        let (count,): (i64,) = sqlx::query_as("SELECT mail_count FROM threads")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn duplicate_is_skipped() {
        let db = db_with_account().await;
        let repo = MailRepo::new(&db);
        repo.upsert_batch(&[parsed("acc", "<dup@x>", "A", None)])
            .await
            .unwrap();
        let (stats, _) = repo
            .upsert_batch(&[parsed("acc", "<dup@x>", "A", None)])
            .await
            .unwrap();
        assert_eq!(stats.skipped_duplicate, 1);
        assert_eq!(stats.inserted, 0);
        assert_eq!(repo.count_for_account("acc").await.unwrap(), 1);
    }
}
