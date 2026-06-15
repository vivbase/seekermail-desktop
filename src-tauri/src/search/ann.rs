//! Semantic search (C2 / GTE, T033).
//!
//! Two-stage retrieval (01 §"Two-stage query pattern"):
//!   1. **SQLite pre-filter** — narrow to candidate `mail_id`s by account / date /
//!      not-archived (capped at [`PRE_FILTER_LIMIT`]).
//!   2. **Vector ANN** — embed the query, run cosine ANN over the vector store,
//!      keep only hits inside the candidate set, aggregate chunks per mail (max
//!      score), apply time decay, threshold, and return the top mails.
//!
//! The default build's vector store is the brute-force cosine backend (T019); the
//! same code drives a real LanceDB backend unchanged because both implement the
//! [`crate::vector::VectorStore`] surface.

use std::collections::{HashMap, HashSet};

use sqlx::Row;
use tracing::Instrument;

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::types::{PageResult, ScoreLabel, SearchResult, SemanticSearchParams};
use crate::util::now_unix;
use crate::vector::AnnFilter;

/// Minimum raw-query length for semantic search (shorter → VALIDATION → UI falls
/// back to keyword search, T033 §3).
const MIN_QUERY_CHARS: usize = 3;
/// Chunk-level ANN fan-out (F_C2, T033 §3).
const ANN_CHUNK_LIMIT: usize = 50;
/// Mails returned after per-mail aggregation (Top-20, T033 §3).
const TOP_MAILS: usize = 20;
/// Default cosine floor (T033 §3).
const DEFAULT_MIN_SCORE: f32 = 0.35;
/// Largest candidate set passed to the ANN filter before random sampling kicks in.
const PRE_FILTER_LIMIT: i64 = 20_000;

/// Run semantic search. Returns `VALIDATION` for too-short queries and an empty
/// page when nothing clears the score threshold.
pub async fn search_semantic(
    state: &AppState,
    params: &SemanticSearchParams,
) -> AppResult<PageResult<SearchResult>> {
    let query = params.query.trim();
    if query.chars().count() < MIN_QUERY_CHARS {
        return Err(AppError::Validation(
            "query too short for semantic search".into(),
        ));
    }
    let min_score = params.min_score.unwrap_or(DEFAULT_MIN_SCORE);
    let limit = params.limit.clamp(1, TOP_MAILS as u32) as usize;
    let offset = params.offset as usize;

    // 1) Embed the query (local ONNX is treated as the "provider" for errors).
    // NB: instrument the future rather than holding an `EnteredSpan` guard across
    // `.await` — an entered span is `!Send`, which would make this command's future
    // `!Send` and break Tauri's `generate_handler!` (which requires `Send`).
    let query_vec = state
        .embedder
        .embed_blocking(query.to_string())
        .instrument(tracing::info_span!("embed_query"))
        .await
        .map_err(|e| AppError::AiUnreachable(format!("query embed failed: {e}")))?;

    // 2) SQLite pre-filter → candidate mail_ids.
    let candidates = prefilter_candidates(state, params)
        .instrument(tracing::info_span!("sqlite_prefilter"))
        .await?;
    if candidates.is_empty() {
        return Ok(PageResult {
            items: vec![],
            total: 0,
            offset: params.offset,
        });
    }
    let candidate_set: HashSet<&str> = candidates.iter().map(|s| s.as_str()).collect();

    // 3) ANN over the vector store, scoped by account/date. The ANN call is
    // synchronous, so `in_scope` enters the span only for the closure (no await).
    let hits = tracing::info_span!("vector_ann").in_scope(|| {
        state.storage.vectors().ann(
            &query_vec,
            ANN_CHUNK_LIMIT,
            AnnFilter {
                account_id: params.account_id.clone(),
                date_from: params.date_from,
                date_to: params.date_to,
            },
        )
    })?;

    // Aggregate chunks → per-mail max cosine, keep only candidates.
    let mut best: HashMap<String, f32> = HashMap::new();
    for h in &hits {
        if !candidate_set.contains(h.mail_id.as_str()) {
            continue;
        }
        let entry = best.entry(h.mail_id.clone()).or_insert(f32::MIN);
        if h.score > *entry {
            *entry = h.score;
        }
    }
    // Threshold gate.
    best.retain(|_, &mut s| s >= min_score);
    if best.is_empty() {
        return Ok(PageResult {
            items: vec![],
            total: 0,
            offset: params.offset,
        });
    }

    // 4) Hydrate + rerank.
    let mut ranked = hydrate(state, &best)
        .instrument(tracing::info_span!("hydrate_rerank"))
        .await?;
    let now = now_unix();
    // Deterministic ranking (M10): score×decay DESC, then date_sent DESC, then
    // mail_id ASC so ties resolve identically across repeated runs.
    ranked.sort_by(|a, b| {
        let sa = a.score * decay(a.date_sent, now);
        let sb = b.score * decay(b.date_sent, now);
        sb.partial_cmp(&sa)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.date_sent.cmp(&a.date_sent))
            .then(a.mail_id.cmp(&b.mail_id))
    });

    // Flat-distribution warning (results all look alike, T033 §3).
    if ranked.len() >= 2 {
        let hi = ranked.first().map(|r| r.score).unwrap_or(0.0);
        let lo = ranked
            .iter()
            .take(TOP_MAILS)
            .next_back()
            .map(|r| r.score)
            .unwrap_or(0.0);
        if (hi - lo) < 0.1 {
            tracing::warn!(
                query_len = query.len(),
                "semantic results have a flat score distribution"
            );
        }
    }

    let total = ranked.len().min(TOP_MAILS) as u32;
    let items: Vec<SearchResult> = ranked.into_iter().skip(offset).take(limit).collect();
    Ok(PageResult {
        items,
        total,
        offset: params.offset,
    })
}

/// Stage 1 — candidate mail_ids (account / date / not-archived), sampled if huge.
async fn prefilter_candidates(
    state: &AppState,
    params: &SemanticSearchParams,
) -> AppResult<Vec<String>> {
    let mut sql = String::from("SELECT id FROM mails WHERE is_archived = 0 AND is_deleted = 0");
    // Account scoping (T112): single account → equality; `account_filter` →
    // subset IN-list; neither → every *active* account (cross-account).
    let account_subset = params
        .account_filter
        .as_ref()
        .filter(|v| !v.is_empty())
        .filter(|_| params.account_id.is_none());
    if params.account_id.is_some() {
        sql.push_str(" AND account_id = ?");
    } else if let Some(ids) = account_subset {
        let placeholders = vec!["?"; ids.len()].join(",");
        sql.push_str(&format!(" AND account_id IN ({placeholders})"));
    } else {
        sql.push_str(" AND account_id IN (SELECT id FROM accounts WHERE is_active = 1)");
    }
    if params.date_from.is_some() {
        sql.push_str(" AND date_sent >= ?");
    }
    if params.date_to.is_some() {
        sql.push_str(" AND date_sent <= ?");
    }
    sql.push_str(" ORDER BY RANDOM() LIMIT ?");

    let mut q = sqlx::query(&sql);
    if let Some(acc) = &params.account_id {
        q = q.bind(acc);
    } else if let Some(ids) = account_subset {
        for id in ids {
            q = q.bind(id);
        }
    }
    if let Some(from) = params.date_from {
        q = q.bind(from);
    }
    if let Some(to) = params.date_to {
        q = q.bind(to);
    }
    q = q.bind(PRE_FILTER_LIMIT);

    let rows = q
        .fetch_all(state.storage.db().pool())
        .await
        .map_err(super::map_err)?;
    if rows.len() as i64 >= PRE_FILTER_LIMIT {
        tracing::warn!(
            limit = PRE_FILTER_LIMIT,
            "candidate set hit the pre-filter cap; sampled"
        );
    }
    Ok(rows.iter().map(|r| r.get::<String, _>("id")).collect())
}

/// Fetch `SearchResult` rows for the scored mails (score = cosine; no highlights).
async fn hydrate(state: &AppState, best: &HashMap<String, f32>) -> AppResult<Vec<SearchResult>> {
    let ids: Vec<&String> = best.keys().collect();
    let placeholders = vec!["?"; ids.len()].join(",");
    let sql = format!(
        "SELECT id, account_id, subject, from_name, from_email, date_sent, COALESCE(snippet,'') AS snippet \
         FROM mails WHERE id IN ({placeholders})"
    );
    let mut q = sqlx::query(&sql);
    for id in &ids {
        q = q.bind(*id);
    }
    let rows = q
        .fetch_all(state.storage.db().pool())
        .await
        .map_err(super::map_err)?;
    Ok(rows
        .iter()
        .map(|r| {
            let id: String = r.get("id");
            let score = *best.get(&id).unwrap_or(&0.0);
            SearchResult {
                mail_id: id,
                account_id: r.get("account_id"),
                subject: r.get("subject"),
                from_name: r.get("from_name"),
                from_email: r.get("from_email"),
                date_sent: r.get("date_sent"),
                snippet: r.get("snippet"),
                score: score.clamp(0.0, 1.0),
                score_label: ScoreLabel::from_score(score),
                highlights: vec![],
            }
        })
        .collect())
}

/// Recency multiplier matching the keyword ranker (T032/T033 §3).
fn decay(date_sent: i64, now: i64) -> f32 {
    let age = now - date_sent;
    if age < 2_592_000 {
        1.2
    } else if age < 7_776_000 {
        1.1
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::VectorRow;

    fn sem(query: &str) -> SemanticSearchParams {
        SemanticSearchParams {
            query: query.into(),
            account_id: None,
            account_filter: None,
            date_from: None,
            date_to: None,
            min_score: None,
            limit: 20,
            offset: 0,
        }
    }

    /// Seed an account + a mail row + its chunk vector(s), embedding `text`.
    async fn seed_mail(state: &AppState, id: &str, acc: &str, text: &str) {
        let db = state.storage.db();
        sqlx::query(
            "INSERT OR IGNORE INTO accounts (id,email,display_name,provider,color_token,badge_label,created_at,updated_at) \
             VALUES (?, ?, 'X', 'imap', 'slate', 'W', 0, 0)",
        )
        .bind(acc)
        .bind(format!("{acc}@x.com"))
        .execute(db.pool())
        .await
        .ok();
        sqlx::query(
            "INSERT INTO mails (id,account_id,message_id,subject,from_email,to_addrs,date_sent,date_received,body_text,snippet,embedding_status,created_at,updated_at) \
             VALUES (?, ?, ?, ?, 'a@x.com', '[]', ?, ?, ?, ?, 'indexed', 0, 0)",
        )
        .bind(id)
        .bind(acc)
        .bind(format!("<{id}@x>"))
        .bind(text)
        .bind(now_unix())
        .bind(now_unix())
        .bind(text)
        .bind(text)
        .execute(db.pool())
        .await
        .unwrap();
        let v = state.embedder.embed(text).unwrap();
        state
            .storage
            .vectors()
            .upsert(&[VectorRow {
                chunk_id: format!("{id}:0"),
                mail_id: id.into(),
                chunk_index: 0,
                account_id: acc.into(),
                from_email: "a@x.com".into(),
                date_sent: now_unix(),
                subject: text.into(),
                snippet: text.into(),
                embedding_model: "bge-m3".into(),
                vector: v,
            }])
            .unwrap();
    }

    #[tokio::test]
    async fn short_query_is_validation_error() {
        let (state, _rx) = AppState::test_state().await;
        let r = search_semantic(&state, &sem("hi")).await;
        assert!(matches!(r.unwrap_err(), AppError::Validation(_)));
    }

    #[tokio::test]
    async fn finds_semantically_related_mail() {
        let (state, _rx) = AppState::test_state().await;
        seed_mail(
            &state,
            "m1",
            "a",
            "the quarterly budget report and revenue figures",
        )
        .await;
        seed_mail(&state, "m2", "a", "lunch plans for saturday at the cafe").await;
        let res = search_semantic(&state, &sem("budget revenue report"))
            .await
            .unwrap();
        assert!(!res.items.is_empty());
        assert_eq!(res.items[0].mail_id, "m1");
    }

    #[tokio::test]
    async fn account_filter_isolates_results() {
        let (state, _rx) = AppState::test_state().await;
        seed_mail(&state, "m1", "a", "budget report numbers").await;
        seed_mail(&state, "m2", "b", "budget report numbers").await;
        let mut p = sem("budget report");
        p.account_id = Some("b".into());
        let res = search_semantic(&state, &p).await.unwrap();
        assert!(res.items.iter().all(|r| r.account_id == "b"));
    }

    #[tokio::test]
    async fn cross_account_returns_all_accounts_deterministically() {
        let (state, _rx) = AppState::test_state().await;
        seed_mail(&state, "m1", "a", "quarterly budget revenue report figures").await;
        seed_mail(&state, "m2", "b", "quarterly budget revenue report figures").await;
        seed_mail(&state, "m3", "c", "quarterly budget revenue report figures").await;

        // account_id = None → all accounts.
        let r1 = search_semantic(&state, &sem("budget revenue report"))
            .await
            .unwrap();
        let accts: std::collections::HashSet<&str> =
            r1.items.iter().map(|i| i.account_id.as_str()).collect();
        assert!(accts.len() >= 2, "cross-account search spans accounts");

        // M10: three consecutive runs return an identical Top-N order.
        let ids1: Vec<String> = r1.items.iter().map(|i| i.mail_id.clone()).collect();
        let r2 = search_semantic(&state, &sem("budget revenue report"))
            .await
            .unwrap();
        let r3 = search_semantic(&state, &sem("budget revenue report"))
            .await
            .unwrap();
        let ids2: Vec<String> = r2.items.iter().map(|i| i.mail_id.clone()).collect();
        let ids3: Vec<String> = r3.items.iter().map(|i| i.mail_id.clone()).collect();
        assert_eq!(ids1, ids2);
        assert_eq!(ids2, ids3);
    }

    #[tokio::test]
    async fn account_filter_restricts_to_subset() {
        let (state, _rx) = AppState::test_state().await;
        seed_mail(&state, "m1", "a", "budget report numbers").await;
        seed_mail(&state, "m2", "b", "budget report numbers").await;
        seed_mail(&state, "m3", "c", "budget report numbers").await;

        let mut p = sem("budget report");
        p.account_filter = Some(vec!["a".into(), "b".into()]);
        let res = search_semantic(&state, &p).await.unwrap();
        assert!(!res.items.is_empty());
        assert!(
            res.items
                .iter()
                .all(|r| r.account_id == "a" || r.account_id == "b"),
            "account_filter must exclude account c"
        );
    }

    #[tokio::test]
    async fn high_threshold_returns_empty() {
        let (state, _rx) = AppState::test_state().await;
        seed_mail(
            &state,
            "m1",
            "a",
            "completely unrelated text about gardening",
        )
        .await;
        let mut p = sem("astrophysics quantum mechanics");
        p.min_score = Some(0.99);
        let res = search_semantic(&state, &p).await.unwrap();
        assert!(res.items.is_empty());
    }
}
