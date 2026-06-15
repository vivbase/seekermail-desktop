//! Agent presence + identity commands (T094).
//!
//! Thin boundary (03 §1): one repo call, map `AppError → IpcError`. Presence is a
//! pure derivation over `sync_state` + recent `ai_drafts` — nothing is persisted.

use tauri::State;

use crate::error::IpcError;
use crate::state::AppState;
use crate::storage::AccountRepo;
use crate::types::AgentStatus;

/// Current Agent presence for every active account (T094, F_I2 §4.2).
#[tauri::command]
pub async fn get_agent_statuses(state: State<'_, AppState>) -> Result<Vec<AgentStatus>, IpcError> {
    AccountRepo::new(state.storage.db())
        .agent_statuses()
        .await
        .map_err(IpcError::from)
}
