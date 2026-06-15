//! Agent-IM (TEAM) channel commands (T092).
//!
//! Thin boundary (03 §1): deserialize → one repo call → map `AppError → IpcError`.
//! The only extra is the detached retention sweep after a successful insert, which
//! must never block the command's return (F_I2 §5).

use tauri::State;

use crate::error::IpcError;
use crate::state::AppState;
use crate::storage::im_repo;
use crate::types::{ImMessage, PageResult};

/// Post one message to the shared TEAM channel (T092). `channel_id` must be
/// `"main"` — the data-layer no-private-chats guard returns `VALIDATION` otherwise.
#[tauri::command]
pub async fn post_im_message(
    state: State<'_, AppState>,
    channel_id: String,
    sender_type: String,
    sender_id: String,
    message_type: String,
    content: String,
    linked_email_id: Option<String>,
) -> Result<ImMessage, IpcError> {
    let msg = im_repo::insert_message(
        state.storage.db(),
        &channel_id,
        &sender_type,
        &sender_id,
        &message_type,
        &content,
        linked_email_id.as_deref(),
        None,
    )
    .await
    .map_err(IpcError::from)?;

    // Retention sweep runs detached so it never blocks the command return (F_I2 §5).
    let db = state.storage.db().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = im_repo::purge_old(&db).await {
            tracing::warn!(error = %e, "im_messages retention purge failed");
        }
    });

    Ok(msg)
}

/// List TEAM channel messages oldest-first with pagination + optional filters.
#[tauri::command]
pub async fn list_im_messages(
    state: State<'_, AppState>,
    sender_id: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<PageResult<ImMessage>, IpcError> {
    im_repo::list_messages(
        state.storage.db(),
        sender_id.as_deref(),
        status.as_deref(),
        limit,
        offset,
    )
    .await
    .map_err(IpcError::from)
}

/// Mark one message read (idempotent — keeps the first `read_at`).
#[tauri::command]
pub async fn mark_im_message_read(state: State<'_, AppState>, id: String) -> Result<(), IpcError> {
    im_repo::mark_read(state.storage.db(), &id)
        .await
        .map_err(IpcError::from)
}

/// Count of queries still awaiting a human answer (T101 — the sidebar TEAM
/// badge). The full query lifecycle commands land with v0.6 (T095–T097); v0.5
/// only needs this read.
#[tauri::command]
pub async fn count_pending_queries(state: State<'_, AppState>) -> Result<u32, IpcError> {
    crate::storage::query_repo::count_pending(state.storage.db())
        .await
        .map(|n| n as u32)
        .map_err(IpcError::from)
}
