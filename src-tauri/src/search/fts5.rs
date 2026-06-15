//! FTS5 keyword-search execution + index maintenance (T032, F_C1).
//!
//! One mail maps to exactly one `mails_fts` row (the `001_init.sql` triggers keep
//! them in lock-step), so a `MATCH` returns each mail at most once — no `GROUP BY`
//! dedup is needed. Ranking is `-bm25() × time_decay`; the raw BM25 is normalised
//! to 0–1 within the returned page (F_C1 §5, T032 §6).

use sqlx::{Row, SqlitePool};

use crate::error::{AppError, AppResult};
use crate::search::dsl::parse_keyword_query;
use crate::types::{KeywordSearchParams, PageResult, ScoreLabel, SearchResult};

/// Minimum raw-query length before we run FTS (shorter → empty result, no query).
const MIN_QUERY_LEN: usize = 3;
/// Hard cap on page size (02 §Pagination).
const MAX_LIMIT: u32 = 200;

/// A bound value for the dynamically assembled filter clause.
enum Bind {
    Text(String),
    Int(i64),
}

/// Execute a keyword search. Returns an empty page for queries shorter than
/// [`MIN_QUERY_LEN`] without touching FTS5 (T032 §3).
pub async fn search_keyword_fts5(
    db: &SqlitePool,
    params: &KeywordSearchParams,
) -> AppResult<PageResult<SearchResult>> {
    let limit = params.limit.clamp(1, MAX_LIMIT);
    let offset = params.offset;

    if params.query.trim().chars().count() < MIN_QUERY_LEN {
        return Ok(PageResult {
            items: vec![],
            total: 0,
            offset,
        });
    }

    let dsl = parse_keyword_query(&params.query);

    // Shared WHERE fragments + binds (everything except the FTS MATCH).
    let mut clauses: Vec<String> = vec!["m.is_deleted = 0".into()];
    let mut binds: Vec<Bind> = Vec::new();
    if let Some(acc) = &params.account_id {
        clauses.push("m.account_id = ?".into());
        binds.push(Bind::Text(acc.clone()));
    }
    if let Some(from) = params.date_from {
        clauses.push("m.date_sent >= ?".into());
        binds.push(Bind::Int(from));
    }
    if let Some(to) = params.date_to {
        clauses.push("m.date_sent <= ?".into());
        binds.push(Bind::Int(to));
    }
    // `in:` from the DSL overrides the params.folder filter.
    if let Some(folder) = dsl.folder.clone().or_else(|| params.folder.clone()) {
        clauses.push("m.folder = ?".into());
        binds.push(Bind::Text(folder));
    }
    if let Some(to_addr) = &dsl.to_filter {
        clauses.push("lower(m.to_addrs) LIKE ?".into());
        binds.push(Bind::Text(format!("%{to_addr}%")));
    }
    if dsl.has_attachment {
        clauses.push("m.has_attachments = 1".into());
    }
    let where_sql = clauses.join(" AND ");

    if dsl.fts_match.is_empty() {
        // Filter-only query (e.g. `has:attachment`) — no FTS, order by recency.
        return filter_only(db, &where_sql, binds, limit, offset).await;
    }

    let decay = time_decay_sql("m.date_sent");

    // ── total ────────────────────────────────────────────────────────────────
    let count_sql = format!(
        "SELECT count(*) FROM mails_fts JOIN mails m ON m.rowid = mails_fts.rowid \
         WHERE mails_fts MATCH ? AND {where_sql}"
    );
    let mut cq = sqlx::query_scalar::<_, i64>(&count_sql).bind(&dsl.fts_match);
    for b in &binds {
        cq = match b {
            Bind::Text(s) => cq.bind(s),
            Bind::Int(i) => cq.bind(i),
        };
    }
    let total: i64 = cq.fetch_one(db).await.map_err(super::map_err)?;

    // ── page ─────────────────────────────────────────────────────────────────
    let page_sql = format!(
        "SELECT m.id, m.account_id, m.subject, m.from_name, m.from_email, m.date_sent, \
             COALESCE(m.snippet, '') AS snippet, \
             snippet(mails_fts, 0, '<mark>', '</mark>', '…', 12) AS hl_subject, \
             snippet(mails_fts, 1, '<mark>', '</mark>', '…', 20) AS hl_body, \
             snippet(mails_fts, 2, '<mark>', '</mark>', '…', 8)  AS hl_from, \
             bm25(mails_fts) AS rank \
         FROM mails_fts JOIN mails m ON m.rowid = mails_fts.rowid \
         WHERE mails_fts MATCH ? AND {where_sql} \
         ORDER BY (-bm25(mails_fts)) * {decay} DESC, m.date_sent DESC, m.id ASC \
         LIMIT ? OFFSET ?"
    );
    let mut pq = sqlx::query(&page_sql).bind(&dsl.fts_match);
    for b in &binds {
        pq = match b {
            Bind::Text(s) => pq.bind(s),
            Bind::Int(i) => pq.bind(i),
        };
    }
    pq = pq.bind(limit as i64).bind(offset as i64);
    let rows = pq.fetch_all(db).await.map_err(super::map_err)?;

    // Normalise BM25 within the page: raw = -bm25 (higher = better).
    let max_raw = rows
        .iter()
        .map(|r| -r.get::<f64, _>("rank"))
        .fold(f64::MIN, f64::max)
        .max(1e-9);

    let items = rows
        .iter()
        .map(|r| {
            let raw = -r.get::<f64, _>("rank");
            let score = (raw / max_raw).clamp(0.0, 1.0) as f32;
            let highlights = collect_highlights(&[
                r.get::<Option<String>, _>("hl_subject"),
                r.get::<Option<String>, _>("hl_body"),
                r.get::<Option<String>, _>("hl_from"),
            ]);
            SearchResult {
                mail_id: r.get("id"),
                account_id: r.get("account_id"),
                subject: r.get("subject"),
                from_name: r.get("from_name"),
                from_email: r.get("from_email"),
                date_sent: r.get("date_sent"),
                snippet: r.get("snippet"),
                score,
                score_label: ScoreLabel::from_score(score),
                highlights,
            }
        })
        .collect();

    Ok(PageResult {
        items,
        total: total as u32,
        offset,
    })
}

/// Filter-only path (DSL produced no FTS terms): order by recency, neutral score.
async fn filter_only(
    db: &SqlitePool,
    where_sql: &str,
    binds: Vec<Bind>,
    limit: u32,
    offset: u32,
) -> AppResult<PageResult<SearchResult>> {
    let count_sql = format!("SELECT count(*) FROM mails m WHERE {where_sql}");
    let mut cq = sqlx::query_scalar::<_, i64>(&count_sql);
    for b in &binds {
        cq = match b {
            Bind::Text(s) => cq.bind(s),
            Bind::Int(i) => cq.bind(i),
        };
    }
    let total: i64 = cq.fetch_one(db).await.map_err(super::map_err)?;

    let page_sql = format!(
        "SELECT m.id, m.account_id, m.subject, m.from_name, m.from_email, m.date_sent, \
             COALESCE(m.snippet, '') AS snippet \
         FROM mails m WHERE {where_sql} ORDER BY m.date_sent DESC, m.id ASC LIMIT ? OFFSET ?"
    );
    let mut pq = sqlx::query(&page_sql);
    for b in &binds {
        pq = match b {
            Bind::Text(s) => pq.bind(s),
            Bind::Int(i) => pq.bind(i),
        };
    }
    pq = pq.bind(limit as i64).bind(offset as i64);
    let rows = pq.fetch_all(db).await.map_err(super::map_err)?;

    let items = rows
        .iter()
        .map(|r| SearchResult {
            mail_id: r.get("id"),
            account_id: r.get("account_id"),
            subject: r.get("subject"),
            from_name: r.get("from_name"),
            from_email: r.get("from_email"),
            date_sent: r.get("date_sent"),
            snippet: r.get("snippet"),
            score: 0.5,
            score_label: ScoreLabel::Mid,
            highlights: vec![],
        })
        .collect();
    Ok(PageResult {
        items,
        total: total as u32,
        offset,
    })
}

/// SQL expression for the time-decay multiplier (T032 §6): ≤30d ×1.2, ≤90d ×1.1.
fn time_decay_sql(col: &str) -> String {
    format!(
        "(CASE WHEN {col} > (strftime('%s','now') - 2592000) THEN 1.2 \
               WHEN {col} > (strftime('%s','now') - 7776000) THEN 1.1 ELSE 1.0 END)"
    )
}

/// First 3 distinct non-empty highlight fragments.
fn collect_highlights(cols: &[Option<String>]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in cols.iter().flatten() {
        let trimmed = c.trim();
        if trimmed.contains("<mark>") && !out.iter().any(|h| h == trimmed) {
            out.push(trimmed.to_string());
        }
        if out.len() == 3 {
            break;
        }
    }
    out
}

/// Rebuild the FTS5 index from the `mails` content table (H2 corruption recovery,
/// T032 §3). Idempotent.
pub async fn rebuild_fts_index(db: &SqlitePool) -> AppResult<()> {
    sqlx::query("INSERT INTO mails_fts(mails_fts) VALUES ('rebuild')")
        .execute(db)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("fts rebuild: {e}")))?;
    Ok(())
}

/// One attachment-FTS hit (T109 §3c internal API). T110 wraps this into the
/// `AttachmentHit` IPC type for the search panel.
#[derive(Debug, Clone)]
pub struct AttachmentSearchHit {
    pub attachment_id: String,
    pub mail_id: String,
    pub filename: String,
    /// FTS5 `<mark>`-highlighted excerpt of the extracted text.
    pub excerpt: String,
    /// Page-normalised relevance in 0..1 (higher = better).
    pub score: f32,
}

/// Search the attachment full-text index (T109). `account_id = None` searches
/// every account (cross-account). Ordering is deterministic on ties
/// (`bm25 ASC, a.id ASC`) so repeat queries are stable (M10).
pub async fn search_attachments_fts(
    db: &SqlitePool,
    query: &str,
    account_id: Option<&str>,
    limit: i64,
) -> AppResult<Vec<AttachmentSearchHit>> {
    if query.trim().chars().count() < MIN_QUERY_LEN {
        return Ok(vec![]);
    }
    let dsl = parse_keyword_query(query);
    if dsl.fts_match.is_empty() {
        return Ok(vec![]);
    }

    let acc_clause = if account_id.is_some() {
        " AND a.account_id = ?"
    } else {
        ""
    };
    let sql = format!(
        "SELECT a.id AS attachment_id, a.mail_id AS mail_id, a.filename AS filename, \
             snippet(attachments_fts, 1, '<mark>', '</mark>', '…', 20) AS excerpt, \
             bm25(attachments_fts) AS rank \
         FROM attachments_fts JOIN attachments a ON a.rowid = attachments_fts.rowid \
         WHERE attachments_fts MATCH ?{acc_clause} \
         ORDER BY bm25(attachments_fts) ASC, a.id ASC \
         LIMIT ?"
    );
    let mut q = sqlx::query(&sql).bind(&dsl.fts_match);
    if let Some(acc) = account_id {
        q = q.bind(acc);
    }
    q = q.bind(limit.clamp(1, MAX_LIMIT as i64));
    let rows = q.fetch_all(db).await.map_err(super::map_err)?;

    // Normalise -bm25 within the page (higher = better), matching mail scoring.
    let max_raw = rows
        .iter()
        .map(|r| -r.get::<f64, _>("rank"))
        .fold(f64::MIN, f64::max)
        .max(1e-9);
    Ok(rows
        .iter()
        .map(|r| {
            let raw = -r.get::<f64, _>("rank");
            AttachmentSearchHit {
                attachment_id: r.get("attachment_id"),
                mail_id: r.get("mail_id"),
                filename: r.get("filename"),
                excerpt: r.get::<Option<String>, _>("excerpt").unwrap_or_default(),
                score: (raw / max_raw).clamp(0.0, 1.0) as f32,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Db;

    async fn seed() -> Db {
        let db = Db::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        let pool = db.pool();
        sqlx::query(
            "INSERT INTO accounts (id,email,display_name,provider,color_token,badge_label,created_at,updated_at) \
             VALUES ('a','me@x.com','Me','imap','slate','W',0,0),('b','two@x.com','Two','imap','sage','P',0,0)",
        )
        .execute(pool)
        .await
        .unwrap();
        let now = crate::util::now_unix();
        for (id, acc, subj, body, from, att, folder) in [
            (
                "m1",
                "a",
                "Q4 budget review",
                "The quarterly budget report is attached",
                "alice@example.com",
                1,
                "INBOX",
            ),
            (
                "m2",
                "a",
                "Lunch plans",
                "Are we still on for lunch saturday",
                "bob@example.com",
                0,
                "INBOX",
            ),
            (
                "m3",
                "a",
                "Budget follow-up",
                "Following up on the budget report numbers",
                "alice@example.com",
                0,
                "INBOX",
            ),
            (
                "m4",
                "b",
                "Other account budget",
                "Different account budget memo",
                "carol@example.com",
                0,
                "Sent",
            ),
        ] {
            sqlx::query(
                "INSERT INTO mails (id,account_id,message_id,subject,from_name,from_email,to_addrs,date_sent,date_received,body_text,snippet,has_attachments,folder,created_at,updated_at) \
                 VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(id).bind(acc).bind(format!("<{id}@x>")).bind(subj).bind("Sender").bind(from)
            .bind("[{\"email\":\"team@x.com\"}]").bind(now).bind(now).bind(body).bind(body)
            .bind(att).bind(folder).bind(now).bind(now)
            .execute(pool).await.unwrap();
        }
        db
    }

    fn params(query: &str) -> KeywordSearchParams {
        KeywordSearchParams {
            query: query.into(),
            account_id: None,
            date_from: None,
            date_to: None,
            folder: None,
            limit: 50,
            offset: 0,
        }
    }

    #[tokio::test]
    async fn plain_query_returns_highlighted_hits() {
        let db = seed().await;
        let res = search_keyword_fts5(db.pool(), &params("budget"))
            .await
            .unwrap();
        assert!(res.total >= 2);
        assert!(res
            .items
            .iter()
            .any(|r| r.highlights.iter().any(|h| h.contains("<mark>"))));
    }

    #[tokio::test]
    async fn short_query_is_empty_no_fts() {
        let db = seed().await;
        let res = search_keyword_fts5(db.pool(), &params("ab")).await.unwrap();
        assert_eq!(res.total, 0);
        assert!(res.items.is_empty());
    }

    #[tokio::test]
    async fn account_filter_scopes_results() {
        let db = seed().await;
        let mut p = params("budget");
        p.account_id = Some("b".into());
        let res = search_keyword_fts5(db.pool(), &p).await.unwrap();
        assert!(res.items.iter().all(|r| r.account_id == "b"));
        assert_eq!(res.total, 1);
    }

    #[tokio::test]
    async fn has_attachment_filter_only() {
        let db = seed().await;
        let res = search_keyword_fts5(db.pool(), &params("has:attachment"))
            .await
            .unwrap();
        assert!(res.items.iter().all(|r| r.mail_id == "m1"));
        assert_eq!(res.total, 1);
    }

    #[tokio::test]
    async fn from_field_filters_sender() {
        let db = seed().await;
        let res = search_keyword_fts5(db.pool(), &params("from:alice@example.com"))
            .await
            .unwrap();
        assert!(res.total >= 1);
        assert!(res
            .items
            .iter()
            .all(|r| r.from_email == "alice@example.com"));
    }

    #[tokio::test]
    async fn rebuild_is_idempotent() {
        let db = seed().await;
        rebuild_fts_index(db.pool()).await.unwrap();
        rebuild_fts_index(db.pool()).await.unwrap();
        let res = search_keyword_fts5(db.pool(), &params("budget"))
            .await
            .unwrap();
        assert!(res.total >= 2, "index still serves after rebuild");
    }

    #[tokio::test]
    async fn cross_account_search_is_deterministic_and_spans_accounts() {
        let db = seed().await;
        // account_id = None → cross-account (T111).
        let p = params("budget");
        let r1 = search_keyword_fts5(db.pool(), &p).await.unwrap();
        let accts: std::collections::HashSet<&str> =
            r1.items.iter().map(|i| i.account_id.as_str()).collect();
        assert!(accts.len() >= 2, "cross-account spans >1 account");

        // M10: three runs → identical Top-N order (deterministic tiebreak).
        let ids = |r: &PageResult<SearchResult>| -> Vec<String> {
            r.items.iter().map(|i| i.mail_id.clone()).collect()
        };
        let r2 = search_keyword_fts5(db.pool(), &p).await.unwrap();
        let r3 = search_keyword_fts5(db.pool(), &p).await.unwrap();
        assert_eq!(ids(&r1), ids(&r2));
        assert_eq!(ids(&r2), ids(&r3));
    }

    #[tokio::test]
    async fn attachment_fts_respects_account_scope() {
        let db = seed().await; // accounts a, b; m1∈a, m4∈b
        insert_indexed_attachment(
            db.pool(),
            "att-a",
            "m1",
            "a",
            "master service contract terms",
        )
        .await;
        insert_indexed_attachment(
            db.pool(),
            "att-b",
            "m4",
            "b",
            "master service contract terms",
        )
        .await;

        let all = search_attachments_fts(db.pool(), "contract", None, 10)
            .await
            .unwrap();
        assert!(all.len() >= 2, "cross-account attachment search");

        let only_a = search_attachments_fts(db.pool(), "contract", Some("a"), 10)
            .await
            .unwrap();
        assert!(!only_a.is_empty());
        assert!(
            only_a.iter().all(|h| h.attachment_id == "att-a"),
            "account scope must exclude account b's attachment"
        );
    }

    /// Insert an attachment with extracted text and flip it to `indexed` so the
    /// migration-012 FTS trigger fills `attachments_fts`.
    async fn insert_indexed_attachment(
        db: &SqlitePool,
        id: &str,
        mail_id: &str,
        account_id: &str,
        text: &str,
    ) {
        sqlx::query(
            "INSERT INTO attachments (id, mail_id, account_id, filename, content_type, size_bytes, \
             downloaded, local_path, is_inline, extracted_text, extraction_status, created_at) \
             VALUES (?, ?, ?, ?, 'text/plain', 10, 1, ?, 0, ?, 'pending', 0)",
        )
        .bind(id)
        .bind(mail_id)
        .bind(account_id)
        .bind(format!("{id}.txt"))
        .bind(format!("{account_id}/x/{id}.txt"))
        .bind(text)
        .execute(db)
        .await
        .unwrap();
        sqlx::query("UPDATE attachments SET extraction_status = 'indexed' WHERE id = ?")
            .bind(id)
            .execute(db)
            .await
            .unwrap();
    }
}
