//! Export commands (T052) — thin wrappers over `crate::exporter`.
//!
//! `start_export` returns a `task_id` immediately; progress streams via the
//! `export:*` events. `cancel_export` flips the task's cancel flag (partial
//! files stay on disk, F_H2 §4.1). `open_export_output` reveals the finished
//! bundle's directory in Finder — path comes from the task registry, never from
//! the webview, so arbitrary paths can't be opened.

use tauri::State;

use crate::error::{AppError, AppResult, IpcError};
use crate::exporter;
use crate::state::AppState;
use crate::types::StartExportParams;

async fn do_open_export_output(task_id: &str) -> AppResult<()> {
    let handle = exporter::handle_for(task_id).ok_or(AppError::NotFound)?;
    let dir = handle.output_dir;
    if !dir.is_dir() {
        return Err(AppError::NotFound);
    }
    // macOS only in v0.4 (T052 §4); other platforms get a graceful error.
    if cfg!(target_os = "macos") {
        std::process::Command::new("open")
            .arg(&dir)
            .spawn()
            .map_err(|e| AppError::FsPermission(format!("open export dir: {e}")))?;
        Ok(())
    } else {
        Err(AppError::Validation(
            "opening the file manager is macOS-only in v0.4".into(),
        ))
    }
}

/// Start a background export; returns the task id for event correlation.
#[tauri::command]
pub async fn start_export(
    state: State<'_, AppState>,
    params: StartExportParams,
) -> Result<String, IpcError> {
    exporter::spawn_export((*state).clone(), params)
        .await
        .map_err(IpcError::from)
}

/// Request cancellation of a running export.
#[tauri::command]
pub async fn cancel_export(task_id: String) -> Result<(), IpcError> {
    exporter::request_cancel(&task_id).map_err(IpcError::from)
}

/// Reveal a finished export bundle in the system file manager.
#[tauri::command]
pub async fn open_export_output(task_id: String) -> Result<(), IpcError> {
    do_open_export_output(&task_id)
        .await
        .map_err(IpcError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ErrorCode;

    #[tokio::test]
    async fn open_unknown_task_is_not_found() {
        let err = do_open_export_output("missing").await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::NotFound);
    }
}
