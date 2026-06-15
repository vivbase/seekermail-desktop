//! Search commands (Module C) — keyword (T032), semantic (T033), saved (T035).
//!
//! Thin wrappers per the command convention (03 §1): deserialize, call one search
//! service entry point, map `AppError → IpcError`. History is recorded as a
//! best-effort side effect (a failed history write never fails the search).

use tauri::State;

use crate::error::IpcError;
use crate::search;
use crate::state::AppState;
use crate::types::{
    AttachmentHit, KeywordSearchParams, PageResult, SaveSearchParams, SavedSearch,
    SearchHistoryItem, SearchMode, SearchResult, SearchWithAttachmentsParams,
    SearchWithAttachmentsResult, SemanticSearchParams,
};

/// FTS5 keyword search (C1).
#[tauri::command]
pub async fn keyword_search(
    state: State<'_, AppState>,
    params: KeywordSearchParams,
) -> Result<PageResult<SearchResult>, IpcError> {
    let db = state.storage.db().pool();
    let page = search::fts5::search_keyword_fts5(db, &params)
        .await
        .map_err(IpcError::from)?;
    let _ = search::record_history(
        db,
        params.account_id.as_deref(),
        &params.query,
        "keyword",
        page.total as i64,
    )
    .await;
    Ok(page)
}

/// Semantic ANN search (C2 / GTE).
#[tauri::command]
pub async fn semantic_search(
    state: State<'_, AppState>,
    params: SemanticSearchParams,
) -> Result<PageResult<SearchResult>, IpcError> {
    let page = search::ann::search_semantic(&state, &params)
        .await
        .map_err(IpcError::from)?;
    let _ = search::record_history(
        state.storage.db().pool(),
        params.account_id.as_deref(),
        &params.query,
        "semantic",
        page.total as i64,
    )
    .await;
    Ok(page)
}

/// Recent searches for the panel history dropdown (T034).
#[tauri::command]
pub async fn get_search_history(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<SearchHistoryItem>, IpcError> {
    search::list_history(state.storage.db().pool(), limit.unwrap_or(20))
        .await
        .map_err(IpcError::from)
}

/// All saved searches (T035).
#[tauri::command]
pub async fn list_saved_searches(state: State<'_, AppState>) -> Result<Vec<SavedSearch>, IpcError> {
    search::list_saved(state.storage.db().pool())
        .await
        .map_err(IpcError::from)
}

/// Create a saved search (T035).
#[tauri::command]
pub async fn save_search(
    state: State<'_, AppState>,
    params: SaveSearchParams,
) -> Result<SavedSearch, IpcError> {
    search::save(state.storage.db().pool(), &params)
        .await
        .map_err(IpcError::from)
}

/// Delete a saved search (T035).
#[tauri::command]
pub async fn delete_saved_search(state: State<'_, AppState>, id: String) -> Result<(), IpcError> {
    search::delete_saved(state.storage.db().pool(), &id)
        .await
        .map_err(IpcError::from)
}

/// Combined keyword/semantic mail search + attachment full-text search (T110).
/// Returns mail-body hits and attachment-origin hits in one round-trip so the
/// search panel can render both. Does NOT write `search_history` (the dedicated
/// keyword/semantic commands own that), so it never double-logs a query.
#[tauri::command]
pub async fn search_with_attachments(
    state: State<'_, AppState>,
    params: SearchWithAttachmentsParams,
) -> Result<SearchWithAttachmentsResult, IpcError> {
    let db = state.storage.db().pool();
    let limit = params.limit.unwrap_or(50).clamp(1, 200);

    // Mail hits: semantic when explicitly requested, keyword for keyword/auto.
    let mail_hits = match params.mode {
        SearchMode::Semantic => {
            let sp = SemanticSearchParams {
                query: params.query.clone(),
                account_id: params.account_id.clone(),
                account_filter: None,
                date_from: params.date_from,
                date_to: params.date_to,
                min_score: None,
                limit: limit as u32,
                offset: 0,
            };
            search::ann::search_semantic(&state, &sp)
                .await
                .map(|p| p.items)
                .unwrap_or_default()
        }
        SearchMode::Keyword | SearchMode::Auto => {
            let kp = KeywordSearchParams {
                query: params.query.clone(),
                account_id: params.account_id.clone(),
                date_from: params.date_from,
                date_to: params.date_to,
                folder: None,
                limit: limit as u32,
                offset: 0,
            };
            search::fts5::search_keyword_fts5(db, &kp)
                .await
                .map(|p| p.items)
                .unwrap_or_default()
        }
    };

    // Attachment hits, enriched with the owning mail's metadata.
    let raw = search::fts5::search_attachments_fts(
        db,
        &params.query,
        params.account_id.as_deref(),
        limit,
    )
    .await
    .unwrap_or_default();
    let mut attachment_hits = Vec::with_capacity(raw.len());
    for h in raw {
        let meta: Option<(String, String, String, i64)> = sqlx::query_as(
            "SELECT a.content_type, m.subject, m.from_email, m.date_sent \
             FROM attachments a JOIN mails m ON m.id = a.mail_id WHERE a.id = ?",
        )
        .bind(&h.attachment_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten();
        let (content_type, mail_subject, mail_from_email, mail_date_sent) =
            meta.unwrap_or_default();
        attachment_hits.push(AttachmentHit {
            attachment_id: h.attachment_id,
            mail_id: h.mail_id,
            filename: h.filename,
            content_type,
            excerpt: h.excerpt,
            score: h.score,
            mail_subject,
            mail_from_email,
            mail_date_sent,
        });
    }

    Ok(SearchWithAttachmentsResult {
        mail_hits,
        attachment_hits,
    })
}
