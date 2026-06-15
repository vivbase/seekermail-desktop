//! GTE index/engine statistics + topic breakdown (Repository & GTE pages).
//!
//! Read-only aggregates over the local store, so they work without live IMAP:
//! indexing coverage from `mails.embedding_status`, today's knowledge lookups /
//! risks from `ai_decisions` / `risk_events`, and sync recency from `sync_state`.
//! The "Top Topics" breakdown was sourced from deal tags, which were removed with
//! the transaction-view feature; it now returns empty until a new source lands.
//!
//! Thin command wrappers per the command convention (03 §1): the SQL lives in the
//! private loaders below and the command maps `AppError → IpcError`.

use anyhow::Error as AnyError;
use sqlx::SqlitePool;
use tauri::State;

use crate::embedding;
use crate::error::{AppResult, IpcError};
use crate::state::AppState;
use crate::types::{GteStats, KnowledgeEntry, TopicCount};

/// Index/engine statistics for the GTE status row + Repository stat strip / engine panel.
#[tauri::command]
pub async fn get_gte_stats(state: State<'_, AppState>) -> Result<GteStats, IpcError> {
    load_gte_stats(state.storage.db().pool())
        .await
        .map_err(IpcError::from)
}

/// Topic counts for the prototype "Top Topics". The deal-tag source was removed
/// with the transaction-view feature, so this currently returns an empty list.
#[tauri::command]
pub async fn get_topic_breakdown(state: State<'_, AppState>) -> Result<Vec<TopicCount>, IpcError> {
    load_topic_breakdown(state.storage.db().pool())
        .await
        .map_err(IpcError::from)
}

/// `COUNT(*)` helper mapping the sqlx error into the crate error type.
async fn count(db: &SqlitePool, sql: &str) -> AppResult<i64> {
    Ok(sqlx::query_scalar::<_, i64>(sql)
        .fetch_one(db)
        .await
        .map_err(AnyError::from)?)
}

/// Start-of-today predicate (UTC) reused across the "today" aggregates.
const TODAY_CUTOFF: &str = "CAST(strftime('%s','now','start of day') AS INTEGER)";

async fn load_gte_stats(db: &SqlitePool) -> AppResult<GteStats> {
    let email_count = count(db, "SELECT COUNT(*) FROM mails WHERE is_deleted = 0").await?;
    let indexed_count = count(
        db,
        "SELECT COUNT(*) FROM mails WHERE is_deleted = 0 AND embedding_status = 'indexed'",
    )
    .await?;
    let queue_pending = count(
        db,
        "SELECT COUNT(*) FROM mails WHERE is_deleted = 0 AND embedding_status = 'pending'",
    )
    .await?;
    let spam_excluded = count(
        db,
        "SELECT COUNT(*) FROM mails WHERE is_deleted = 0 AND embedding_status = 'skipped'",
    )
    .await?;
    let used_today = count(
        db,
        &format!(
            "SELECT COUNT(*) FROM ai_decisions \
             WHERE knowledge_refs <> '[]' AND created_at >= {TODAY_CUTOFF}"
        ),
    )
    .await?;
    let risks_caught = count(
        db,
        &format!("SELECT COUNT(*) FROM risk_events WHERE created_at >= {TODAY_CUTOFF}"),
    )
    .await?;
    let accounts_syncing = count(db, "SELECT COUNT(*) FROM accounts WHERE is_active = 1").await?;

    let model: Option<String> = sqlx::query_scalar::<_, Option<String>>(
        "SELECT embedding_model FROM mails \
         WHERE embedding_model IS NOT NULL ORDER BY embedded_at DESC LIMIT 1",
    )
    .fetch_optional(db)
    .await
    .map_err(AnyError::from)?
    .flatten();

    let index_version: Option<String> = sqlx::query_scalar::<_, String>(
        "SELECT value FROM app_settings WHERE key = 'gte.index_version'",
    )
    .fetch_optional(db)
    .await
    .map_err(AnyError::from)?;

    let last_sync_at: Option<i64> =
        sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(last_sync_at) FROM sync_state")
            .fetch_one(db)
            .await
            .map_err(AnyError::from)?;

    let vector_count = indexed_count;
    let unindexed_count = (email_count - indexed_count).max(0);
    let coverage_pct = if email_count > 0 {
        (indexed_count as f64 / email_count as f64) * 100.0
    } else {
        0.0
    };
    let dimensions = embedding::DIM as i64;
    let storage_bytes = vector_count * dimensions * 4; // 4 bytes per f32 dimension

    Ok(GteStats {
        email_count,
        indexed_count,
        unindexed_count,
        queue_pending,
        spam_excluded,
        vector_count,
        coverage_pct,
        model: model.unwrap_or_else(|| "bge-m3".to_string()),
        dimensions,
        index_version: index_version.unwrap_or_else(|| "v1".to_string()),
        storage_bytes,
        used_today,
        risks_caught,
        accounts_syncing,
        last_sync_at,
    })
}

async fn load_topic_breakdown(_db: &SqlitePool) -> AppResult<Vec<TopicCount>> {
    // The breakdown was built from deal tags (`deals` / `mail_deals`), which were
    // removed with the transaction-view feature. There is no replacement topic
    // signal in the schema yet, so the list is intentionally empty for now.
    Ok(Vec::new())
}

/// Recent indexed mails as "knowledge entries" — GTE recent list + Repository browse.
#[tauri::command]
pub async fn list_knowledge_entries(
    state: State<'_, AppState>,
    account_id: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<KnowledgeEntry>, IpcError> {
    load_knowledge_entries(
        state.storage.db().pool(),
        account_id.as_deref(),
        limit.unwrap_or(20),
    )
    .await
    .map_err(IpcError::from)
}

/// Base row for one indexed mail (before tag/usage enrichment).
type KnowledgeRow = (
    String,      // id
    String,      // account_id
    String,      // color_token
    String,      // badge_label
    String,      // subject
    String,      // excerpt (snippet)
    String,      // body
    String,      // source (from_email)
    String,      // thread subject
    i64,         // date_sent
    Option<i64>, // embedded_at
);

async fn load_knowledge_entries(
    db: &SqlitePool,
    account_id: Option<&str>,
    limit: i64,
) -> AppResult<Vec<KnowledgeEntry>> {
    let rows: Vec<KnowledgeRow> = sqlx::query_as(
        "SELECT m.id, m.account_id, a.color_token, a.badge_label, m.subject, \
                COALESCE(m.snippet, '') AS excerpt, \
                COALESCE(m.body_html, m.body_text, '') AS body, \
                m.from_email AS source, \
                COALESCE(t.subject, m.subject) AS thread, \
                m.date_sent, m.embedded_at \
         FROM mails m \
         JOIN accounts a ON a.id = m.account_id \
         LEFT JOIN threads t ON t.id = m.thread_id \
         WHERE m.is_deleted = 0 AND m.embedding_status = 'indexed' \
           AND (? IS NULL OR m.account_id = ?) \
         ORDER BY m.date_sent DESC LIMIT ?",
    )
    .bind(account_id)
    .bind(account_id)
    .bind(limit.clamp(1, 100))
    .fetch_all(db)
    .await
    .map_err(AnyError::from)?;

    let mut entries = Vec::with_capacity(rows.len());
    for (id, account_id, acct_color, acct_badge, subject, excerpt, body, source, thread, date_sent, indexed_at) in
        rows
    {
        // Topic/category tags came from deal tags, which were removed with the
        // transaction-view feature; knowledge entries carry no tags for now.
        let tags: Vec<String> = Vec::new();

        // Usage: audit decisions whose knowledge_refs JSON array cites this mail id.
        // Mail ids are UUIDs, so a substring match is unambiguous.
        let usage: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT impact, action_description, created_at FROM ai_decisions \
             WHERE knowledge_refs LIKE '%' || ? || '%' ORDER BY created_at DESC",
        )
        .bind(&id)
        .fetch_all(db)
        .await
        .map_err(AnyError::from)?;

        let used_count = usage.len() as i64;
        let (impact, last_used_for, last_used_time) = match usage.first() {
            Some((imp, act, ts)) => (imp.clone(), Some(act.clone()), Some(*ts)),
            None => ("context".to_string(), None, None),
        };

        entries.push(KnowledgeEntry {
            id,
            account_id,
            acct_color,
            acct_badge,
            subject,
            excerpt,
            body,
            tags,
            date_sent,
            used_count,
            impact,
            last_used_for,
            last_used_time,
            source,
            thread,
            indexed_at,
        });
    }

    Ok(entries)
}
