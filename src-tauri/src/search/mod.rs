//! Search (Module C) — keyword (C1), semantic (C2), and saved searches (T035).
//!
//! * [`dsl`] — the keyword query DSL parser (pure).
//! * [`fts5`] — FTS5 execution + index rebuild.
//! * [`ann`] — semantic two-stage retrieval over the vector store.
//! * history + saved-search persistence helpers live here (thin SQL over
//!   `search_history` / `saved_searches`, both frozen in `001_init.sql`).

pub mod ann;
pub mod dsl;
pub mod fts5;

use sqlx::{Row, SqlitePool};

use crate::error::AppError;
use crate::error::AppResult;
use crate::types::{SaveSearchParams, SavedSearch, SearchHistoryItem};
use crate::util::{new_uuid, now_unix};

/// Keep at most this many rows in `search_history` (T032 §3).
const HISTORY_CAP: i64 = 50;

/// Search-layer sqlx error mapping (no special cases beyond storage's).
pub(crate) fn map_err(e: sqlx::Error) -> AppError {
    crate::storage::map_sqlx_err(e)
}

/// Record one executed search and trim history to [`HISTORY_CAP`] rows.
pub async fn record_history(
    db: &SqlitePool,
    account_id: Option<&str>,
    query: &str,
    mode: &str,
    result_count: i64,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO search_history (account_id, query, mode, result_count, created_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(account_id)
    .bind(query)
    .bind(mode)
    .bind(result_count)
    .bind(now_unix())
    .execute(db)
    .await
    .map_err(map_err)?;

    sqlx::query(
        "DELETE FROM search_history WHERE id NOT IN \
             (SELECT id FROM search_history ORDER BY created_at DESC, id DESC LIMIT ?)",
    )
    .bind(HISTORY_CAP)
    .execute(db)
    .await
    .map_err(map_err)?;
    Ok(())
}

/// Recent searches, newest first (T034 history dropdown).
pub async fn list_history(db: &SqlitePool, limit: i64) -> AppResult<Vec<SearchHistoryItem>> {
    let rows = sqlx::query(
        "SELECT id, query, mode, result_count, created_at FROM search_history \
         ORDER BY created_at DESC, id DESC LIMIT ?",
    )
    .bind(limit.clamp(1, HISTORY_CAP))
    .fetch_all(db)
    .await
    .map_err(map_err)?;
    Ok(rows
        .iter()
        .map(|r| SearchHistoryItem {
            id: r.get("id"),
            query: r.get("query"),
            mode: r.get("mode"),
            result_count: r.get("result_count"),
            created_at: r.get("created_at"),
        })
        .collect())
}

/// All saved searches, ordered (T035).
pub async fn list_saved(db: &SqlitePool) -> AppResult<Vec<SavedSearch>> {
    let rows = sqlx::query(
        "SELECT id, account_id, name, query, mode, sort_order, created_at \
         FROM saved_searches ORDER BY sort_order ASC, created_at ASC",
    )
    .fetch_all(db)
    .await
    .map_err(map_err)?;
    Ok(rows.iter().map(row_to_saved).collect())
}

/// Persist a new saved search (T035). `sort_order` defaults to append-at-end.
pub async fn save(db: &SqlitePool, params: &SaveSearchParams) -> AppResult<SavedSearch> {
    if params.name.trim().is_empty() || params.query.trim().is_empty() {
        return Err(AppError::Validation("name and query are required".into()));
    }
    let id = new_uuid();
    let now = now_unix();
    let (next_order,): (i64,) =
        sqlx::query_as("SELECT COALESCE(MAX(sort_order) + 1, 0) FROM saved_searches")
            .fetch_one(db)
            .await
            .map_err(map_err)?;
    sqlx::query(
        "INSERT INTO saved_searches (id, account_id, name, query, mode, sort_order, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&params.account_id)
    .bind(params.name.trim())
    .bind(params.query.trim())
    .bind(&params.mode)
    .bind(next_order)
    .bind(now)
    .execute(db)
    .await
    .map_err(map_err)?;
    Ok(SavedSearch {
        id,
        account_id: params.account_id.clone(),
        name: params.name.trim().to_string(),
        query: params.query.trim().to_string(),
        mode: params.mode.clone(),
        sort_order: next_order as i32,
        created_at: now,
    })
}

/// Delete a saved search by id (T035). No-op if it doesn't exist.
pub async fn delete_saved(db: &SqlitePool, id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM saved_searches WHERE id = ?")
        .bind(id)
        .execute(db)
        .await
        .map_err(map_err)?;
    Ok(())
}

fn row_to_saved(r: &sqlx::sqlite::SqliteRow) -> SavedSearch {
    SavedSearch {
        id: r.get("id"),
        account_id: r.get("account_id"),
        name: r.get("name"),
        query: r.get("query"),
        mode: r.get("mode"),
        sort_order: r.get::<i64, _>("sort_order") as i32,
        created_at: r.get("created_at"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Db;

    #[tokio::test]
    async fn history_caps_at_50() {
        let db = Db::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        for i in 0..60 {
            record_history(db.pool(), None, &format!("q{i}"), "keyword", i)
                .await
                .unwrap();
        }
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM search_history")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(n, HISTORY_CAP);
    }

    #[tokio::test]
    async fn saved_search_crud_roundtrip() {
        let db = Db::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        let s = save(
            db.pool(),
            &SaveSearchParams {
                name: "Unpaid invoices".into(),
                query: "invoice unpaid".into(),
                mode: "semantic".into(),
                account_id: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(list_saved(db.pool()).await.unwrap().len(), 1);
        delete_saved(db.pool(), &s.id).await.unwrap();
        assert!(list_saved(db.pool()).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn save_rejects_blank() {
        let db = Db::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        let r = save(
            db.pool(),
            &SaveSearchParams {
                name: "  ".into(),
                query: "x".into(),
                mode: "keyword".into(),
                account_id: None,
            },
        )
        .await;
        assert!(matches!(r.unwrap_err(), AppError::Validation(_)));
    }
}
