//! I3/I4 proactive-query commands (T096/T099).
//!
//! Thin boundary (03 §1): deserialize → one service call → map error. The
//! suspend/resume + conservative-fallback logic lives in `ai::pipeline::resume`;
//! the reads live in `storage::query_repo`.

use tauri::State;

use crate::ai::pipeline::resume;
use crate::error::IpcError;
use crate::state::AppState;
use crate::storage::query_repo;
use crate::types::PendingQuery;

/// All pending queries (optionally one account), highest-priority first (T099).
#[tauri::command]
pub async fn list_pending_queries(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<Vec<PendingQuery>, IpcError> {
    query_repo::list_pending(state.storage.db(), account_id.as_deref())
        .await
        .map_err(IpcError::from)
}

/// Apply a human answer and resume the AI chain (T096). `FORBIDDEN` if the query
/// is no longer pending.
#[tauri::command]
pub async fn answer_query(
    state: State<'_, AppState>,
    id: String,
    answer: String,
) -> Result<(), IpcError> {
    resume::answer_query(&state, &id, &answer)
        .await
        .map_err(IpcError::from)
}

/// Skip a query, applying the conservative fallback (T096). T4 never truly drops.
#[tauri::command]
pub async fn skip_query(state: State<'_, AppState>, id: String) -> Result<(), IpcError> {
    resume::skip_query(&state, &id)
        .await
        .map_err(IpcError::from)
}
