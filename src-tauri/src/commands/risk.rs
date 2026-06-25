//! Module E risk-event commands (T071).
//!
//! Thin boundary (03 §1): deserialize params → one repo call → map error. The
//! `risk_events` rows are produced by the D1 legal analyzer and the E4 router;
//! the reads + resolve live in [`crate::storage::risk_event_repo`]. These two
//! commands back the T4 risk banner (`useOpenRiskEvents`) and the Report risk
//! panel (`useRiskEvents` / `useResolveRiskEvent`).

use tauri::State;

use crate::error::IpcError;
use crate::state::AppState;
use crate::storage::risk_event_repo;
use crate::types::{ListRiskEventsParams, ResolveRiskParams, RiskEvent};

/// List risk events, highest risk first (T071). `status` defaults to `open`, so
/// the banner and report panel see only live risks.
#[tauri::command]
pub async fn list_risk_events(
    state: State<'_, AppState>,
    params: ListRiskEventsParams,
) -> Result<Vec<RiskEvent>, IpcError> {
    risk_event_repo::list(state.storage.db(), &params)
        .await
        .map_err(IpcError::from)
}

/// Move one risk event to `resolved`/`dismissed` (T071). `FORBIDDEN` when a
/// `dismissed` is requested for a T4 event (non-dismissable); `NOT_FOUND` for an
/// unknown id.
#[tauri::command]
pub async fn resolve_risk_event(
    state: State<'_, AppState>,
    params: ResolveRiskParams,
) -> Result<(), IpcError> {
    risk_event_repo::resolve(state.storage.db(), &params)
        .await
        .map_err(IpcError::from)?;
    // Tell every other window to clear this risk from its T4 banner (WB-16).
    state.events.risk_resolved(&params.id);
    Ok(())
}
