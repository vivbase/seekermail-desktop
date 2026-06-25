//! `inbox_digest_repo` — the rolling inbox-overview cache (migration 022,
//! analysis/54 §3.5, P-4). One current digest per account, written offline by
//! [`crate::ai::memory`] and read instantly on the query path (no AI on read).
//!
//! Free functions over `&Db`, each a single statement.

use sqlx::Row;

use super::{map_sqlx_err, Db};
use crate::error::AppResult;
use crate::util::now_unix;

/// The current cached inbox overview for one account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxDigest {
    pub account_id: String,
    pub digest: String,
    pub thread_count: i64,
    pub unread_count: i64,
    pub model: Option<String>,
    pub generated_at: i64,
    pub updated_at: i64,
}

/// Fields needed to write (insert or refresh) a digest.
#[derive(Debug, Clone)]
pub struct InboxDigestInput {
    pub account_id: String,
    pub digest: String,
    pub thread_count: i64,
    pub unread_count: i64,
    pub model: Option<String>,
}

/// Insert the account's digest, or refresh it in place.
pub async fn upsert(db: &Db, input: &InboxDigestInput) -> AppResult<()> {
    let now = now_unix();
    sqlx::query(
        "INSERT INTO inbox_digest \
             (account_id, digest, thread_count, unread_count, model, generated_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(account_id) DO UPDATE SET \
             digest       = excluded.digest, \
             thread_count = excluded.thread_count, \
             unread_count = excluded.unread_count, \
             model        = excluded.model, \
             generated_at = excluded.generated_at, \
             updated_at   = excluded.updated_at",
    )
    .bind(&input.account_id)
    .bind(&input.digest)
    .bind(input.thread_count)
    .bind(input.unread_count)
    .bind(&input.model)
    .bind(now)
    .bind(now)
    .execute(db.pool())
    .await
    .map_err(map_sqlx_err)?;
    Ok(())
}

/// The account's current digest, if any.
pub async fn get(db: &Db, account_id: &str) -> AppResult<Option<InboxDigest>> {
    let row = sqlx::query(
        "SELECT account_id, digest, thread_count, unread_count, model, generated_at, updated_at \
         FROM inbox_digest WHERE account_id = ?",
    )
    .bind(account_id)
    .fetch_optional(db.pool())
    .await
    .map_err(map_sqlx_err)?;
    Ok(row.map(|r| InboxDigest {
        account_id: r.get("account_id"),
        digest: r.get("digest"),
        thread_count: r.get("thread_count"),
        unread_count: r.get("unread_count"),
        model: r.get("model"),
        generated_at: r.get("generated_at"),
        updated_at: r.get("updated_at"),
    }))
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

    fn input(acc: &str, digest: &str, threads: i64, unread: i64) -> InboxDigestInput {
        InboxDigestInput {
            account_id: acc.into(),
            digest: digest.into(),
            thread_count: threads,
            unread_count: unread,
            model: Some("gpt-4o".into()),
        }
    }

    #[tokio::test]
    async fn upsert_then_get_roundtrips() {
        let db = db().await;
        seed_account(&db, "a").await;
        upsert(
            &db,
            &input(
                "a",
                "Two renewals need sign-off; one invoice is overdue.",
                12,
                4,
            ),
        )
        .await
        .unwrap();
        let got = get(&db, "a").await.unwrap().expect("digest present");
        assert_eq!(
            got.digest,
            "Two renewals need sign-off; one invoice is overdue."
        );
        assert_eq!(got.thread_count, 12);
        assert_eq!(got.unread_count, 4);
    }

    #[tokio::test]
    async fn upsert_refreshes_in_place() {
        let db = db().await;
        seed_account(&db, "a").await;
        upsert(&db, &input("a", "first", 1, 1)).await.unwrap();
        upsert(&db, &input("a", "second", 3, 2)).await.unwrap();
        let got = get(&db, "a").await.unwrap().unwrap();
        assert_eq!(got.digest, "second");
        assert_eq!(got.thread_count, 3);
    }

    #[tokio::test]
    async fn get_absent_is_none() {
        let db = db().await;
        seed_account(&db, "a").await;
        assert!(get(&db, "a").await.unwrap().is_none());
    }
}
