//! Compose-draft commands (T045) — thin wrappers over `storage::draft_repo`.

use tauri::State;

use crate::error::IpcError;
use crate::state::AppState;
use crate::storage::draft_repo;
use crate::types::{Draft, SaveDraftParams};

/// Create or update (autosave) a compose draft (T045).
#[tauri::command]
pub async fn save_draft(
    state: State<'_, AppState>,
    params: SaveDraftParams,
) -> Result<Draft, IpcError> {
    draft_repo::save(state.storage.db(), &params)
        .await
        .map_err(IpcError::from)
}

/// Fetch a draft by id (T045).
#[tauri::command]
pub async fn get_draft(state: State<'_, AppState>, id: String) -> Result<Draft, IpcError> {
    draft_repo::get(state.storage.db(), &id)
        .await
        .map_err(IpcError::from)
}

/// Delete a draft by id (T045). Idempotent.
#[tauri::command]
pub async fn delete_draft(state: State<'_, AppState>, id: String) -> Result<(), IpcError> {
    draft_repo::delete(state.storage.db(), &id)
        .await
        .map_err(IpcError::from)
}
