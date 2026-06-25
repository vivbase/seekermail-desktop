//! `thread_summary_repo` — the precomputed per-thread memory layer (migration
//! 021, analysis/54 §3.5, P-4).
//!
//! One row per thread holds a one-line summary plus key entities, written
//! offline by the summariser ([`crate::ai::memory`]) and read on the query path
//! by the MCE Memory leg. `mail_count` / `latest_date` are snapshotted so a
//! grown or newer thread reads as stale and gets rebuilt; nothing on the read
//! path calls an AI model.
//!
//! Free functions over `&Db` (the `im_repo` convention), each a single
//! statement.

use sqlx::Row;

use super::{map_sqlx_err, Db};
use crate::error::AppResult;
use crate::util::now_unix;

/// One stored thread summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadSummary {
    pub thread_id: String,
    pub account_id: String,
    /// One-line gist of the thread.
    pub summary: String,
    /// Short key-entity tags (people, companies, topics).
    pub key_entities: Vec<String>,
    /// `threads.mail_count` when this summary was produced (staleness check).
    pub mail_count: i64,
    /// `threads.latest_date` when this summary was produced (staleness check).
    pub latest_date: i64,
    /// LLM that produced the summary, if any.
    pub model: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Fields needed to write (insert or refresh) a summary.
#[derive(Debug, Clone)]
pub struct ThreadSummaryInput {
    pub thread_id: String,
    pub account_id: String,
    pub summary: String,
    pub key_entities: Vec<String>,
    pub mail_count: i64,
    pub latest_date: i64,
    pub model: Option<String>,
}

/// Insert a summary, or refresh it in place if the thread already has one.
/// `created_at` is preserved across refreshes.
pub async fn upsert(db: &Db, input: &ThreadSummaryInput) -> AppResult<()> {
    let now = now_unix();
    let entities_json = serde_json::to_string(&input.key_entities).unwrap_or_else(|_| "[]".into());
    sqlx::query(
        "INSERT INTO thread_summaries \
             (thread_id, account_id, summary, key_entities, mail_count, latest_date, model, \
              created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(thread_id) DO UPDATE SET \
             account_id   = excluded.account_id, \
             summary      = excluded.summary, \
             key_entities = excluded.key_entities, \
             mail_count   = excluded.mail_count, \
             latest_date  = excluded.latest_date, \
             model        = excluded.model, \
             updated_at   = excluded.updated_at",
    )
    .bind(&input.thread_id)
    .bind(&input.account_id)
    .bind(&input.summary)
    .bind(&entities_json)
    .bind(input.mail_count)
    .bind(input.latest_date)
    .bind(&input.model)
    .bind(now)
    .bind(now)
    .execute(db.pool())
    .await
    .map_err(map_sqlx_err)?;
    Ok(())
}

/// One thread's summary, if present.
pub async fn get(db: &Db, thread_id: &str) -> AppResult<Option<ThreadSummary>> {
    let row = sqlx::query(
        "SELECT thread_id, account_id, summary, key_entities, mail_count, latest_date, model, \
                created_at, updated_at \
         FROM thread_summaries WHERE thread_id = ?",
    )
    .bind(thread_id)
    .fetch_optional(db.pool())
    .await
    .map_err(map_sqlx_err)?;
    Ok(row.map(row_to_summary))
}

/// The most recently active thread summaries for an account, newest first.
pub async fn list_recent(db: &Db, account_id: &str, limit: i64) -> AppResult<Vec<ThreadSummary>> {
    let rows = sqlx::query(
        "SELECT s.thread_id, s.account_id, s.summary, s.key_entities, s.mail_count, \
                s.latest_date, s.model, s.created_at, s.updated_at \
         FROM thread_summaries s JOIN threads t ON t.id = s.thread_id \
         WHERE s.account_id = ? AND t.is_archived = 0 \
         ORDER BY s.latest_date DESC LIMIT ?",
    )
    .bind(account_id)
    .bind(limit)
    .fetch_all(db.pool())
    .await
    .map_err(map_sqlx_err)?;
    Ok(rows.into_iter().map(row_to_summary).collect())
}

/// How many summaries an account has.
pub async fn count(db: &Db, account_id: &str) -> AppResult<i64> {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM thread_summaries WHERE account_id = ?")
        .bind(account_id)
        .fetch_one(db.pool())
        .await
        .map_err(map_sqlx_err)?;
    Ok(n)
}

/// Thread ids that need a (re)build: active threads with no summary, or whose
/// `mail_count` / `latest_date` no longer match the stored snapshot. Newest
/// first, capped at `limit`.
pub async fn stale_or_missing_threads(
    db: &Db,
    account_id: &str,
    limit: i64,
) -> AppResult<Vec<String>> {
    let rows = sqlx::query(
        "SELECT t.id FROM threads t \
         LEFT JOIN thread_summaries s ON s.thread_id = t.id \
         WHERE t.account_id = ? AND t.is_archived = 0 \
           AND (s.thread_id IS NULL OR s.mail_count <> t.mail_count OR s.latest_date <> t.latest_date) \
         ORDER BY t.latest_date DESC LIMIT ?",
    )
    .bind(account_id)
    .bind(limit)
    .fetch_all(db.pool())
    .await
    .map_err(map_sqlx_err)?;
    Ok(rows.iter().map(|r| r.get::<String, _>("id")).collect())
}

/// Map a row into a [`ThreadSummary`], decoding the entities JSON defensively.
fn row_to_summary(r: sqlx::sqlite::SqliteRow) -> ThreadSummary {
    let entities_json: String = r.get("key_entities");
    let key_entities = serde_json::from_str::<Vec<String>>(&entities_json).unwrap_or_default();
    ThreadSummary {
        thread_id: r.get("thread_id"),
        account_id: r.get("account_id"),
        summary: r.get("summary"),
        key_entities,
        mail_count: r.get("mail_count"),
        latest_date: r.get("latest_date"),
        model: r.get("model"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> Db {
        let db = Db::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    async fn seed_account(db: &Db, id: &str) {
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
                 role_type, role_description, created_at, updated_at) \
             VALUES (?, ?, 'X', 'imap', 'slate', 'W', 'work', NULL, 0, 0)",
        )
        .bind(id)
        .bind(format!("{id}@x.com"))
        .execute(db.pool())
        .await
        .unwrap();
    }

    async fn seed_thread(db: &Db, id: &str, acc: &str, mail_count: i64, latest_date: i64) {
        sqlx::query(
            "INSERT INTO threads (id, account_id, subject, participants, mail_count, unread_count, \
                 latest_date, created_at, updated_at) \
             VALUES (?, ?, 'Subject', '[]', ?, 0, ?, 0, 0)",
        )
        .bind(id)
        .bind(acc)
        .bind(mail_count)
        .bind(latest_date)
        .execute(db.pool())
        .await
        .unwrap();
    }

    fn input(
        thread: &str,
        acc: &str,
        summary: &str,
        mail_count: i64,
        latest_date: i64,
    ) -> ThreadSummaryInput {
        ThreadSummaryInput {
            thread_id: thread.into(),
            account_id: acc.into(),
            summary: summary.into(),
            key_entities: vec!["Acme".into(), "renewal".into()],
            mail_count,
            latest_date,
            model: Some("gpt-4o".into()),
        }
    }

    #[tokio::test]
    async fn upsert_then_get_roundtrips_entities() {
        let db = db().await;
        seed_account(&db, "a").await;
        seed_thread(&db, "t1", "a", 3, 100).await;

        upsert(
            &db,
            &input("t1", "a", "Acme contract renewal in progress.", 3, 100),
        )
        .await
        .unwrap();
        let got = get(&db, "t1").await.unwrap().expect("summary present");
        assert_eq!(got.summary, "Acme contract renewal in progress.");
        assert_eq!(
            got.key_entities,
            vec!["Acme".to_string(), "renewal".to_string()]
        );
        assert_eq!(got.mail_count, 3);
    }

    #[tokio::test]
    async fn upsert_refreshes_in_place() {
        let db = db().await;
        seed_account(&db, "a").await;
        seed_thread(&db, "t1", "a", 3, 100).await;

        upsert(&db, &input("t1", "a", "first", 3, 100))
            .await
            .unwrap();
        upsert(&db, &input("t1", "a", "second", 5, 200))
            .await
            .unwrap();

        let got = get(&db, "t1").await.unwrap().unwrap();
        assert_eq!(got.summary, "second");
        assert_eq!(got.mail_count, 5);
        assert_eq!(count(&db, "a").await.unwrap(), 1, "one row per thread");
    }

    #[tokio::test]
    async fn list_recent_orders_by_latest_date_and_skips_archived() {
        let db = db().await;
        seed_account(&db, "a").await;
        seed_thread(&db, "old", "a", 1, 100).await;
        seed_thread(&db, "new", "a", 1, 300).await;
        upsert(&db, &input("old", "a", "old gist", 1, 100))
            .await
            .unwrap();
        upsert(&db, &input("new", "a", "new gist", 1, 300))
            .await
            .unwrap();

        let recent = list_recent(&db, "a", 10).await.unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].thread_id, "new", "newest first");

        sqlx::query("UPDATE threads SET is_archived = 1 WHERE id = 'old'")
            .execute(db.pool())
            .await
            .unwrap();
        let recent = list_recent(&db, "a", 10).await.unwrap();
        assert_eq!(recent.len(), 1, "archived threads drop out");
    }

    #[tokio::test]
    async fn stale_or_missing_finds_unsummarised_and_changed_threads() {
        let db = db().await;
        seed_account(&db, "a").await;
        seed_thread(&db, "fresh", "a", 2, 100).await;
        seed_thread(&db, "grown", "a", 2, 100).await;
        seed_thread(&db, "never", "a", 1, 50).await;
        upsert(&db, &input("fresh", "a", "ok", 2, 100))
            .await
            .unwrap();
        upsert(&db, &input("grown", "a", "ok", 2, 100))
            .await
            .unwrap();
        // "grown" gains a mail → its snapshot no longer matches.
        sqlx::query("UPDATE threads SET mail_count = 3 WHERE id = 'grown'")
            .execute(db.pool())
            .await
            .unwrap();

        let stale = stale_or_missing_threads(&db, "a", 10).await.unwrap();
        assert!(
            stale.contains(&"grown".to_string()),
            "changed thread is stale"
        );
        assert!(
            stale.contains(&"never".to_string()),
            "unsummarised thread is missing"
        );
        assert!(
            !stale.contains(&"fresh".to_string()),
            "matching snapshot is fresh"
        );
    }
}
