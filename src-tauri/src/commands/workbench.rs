//! Workbench window-management commands (WB-12, Model S — 02 §3 "Module W").
//!
//! SCAFFOLD — authored where Rust cannot be compiled. **Verify with `cargo build`
//! and `cargo test --lib` on the Mac** (KB note: "Rust must be verified on the Mac";
//! targeted `cargo test --lib` avoids Keychain-prompt hangs). No new `AppState`: open
//! windows are tracked by Tauri itself (`app.webview_windows()`).
//!
//! Each window is an independent OS window (no multi-WebView). A new window boots to a
//! single workspace tab via a `?boot=<token>` query the frontend reads on startup —
//! the frontend encodes a URL-safe TabSpec token; this layer only forwards it.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::error::{AppError, IpcError};

/// Backend → every window: the open-window set changed (frontend refreshes its registry view).
const WINDOW_LIST_CHANGED: &str = "workbench:window_list_changed";

/// Monotonic suffix so every window gets a unique, label-safe id.
static WINDOW_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Deserialize, Type)]
pub struct WindowPos {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Serialize, Type)]
pub struct WindowInfo {
    pub window_label: String,
    pub focused: bool,
}

fn internal(e: tauri::Error) -> IpcError {
    IpcError::from(AppError::Internal(anyhow::Error::new(e)))
}

/// Open a new OS window booted to one workspace tab ("open in new window" / detach).
/// `boot` is a URL-safe token produced by the frontend (an encoded TabSpec) and read
/// from `?boot=` on startup. Returns the new window's label. Emits the list-changed event.
#[tauri::command]
pub async fn workbench_open_window(
    app: AppHandle,
    boot: String,
    at: Option<WindowPos>,
) -> Result<String, IpcError> {
    let label = format!("workbench-{}", WINDOW_SEQ.fetch_add(1, Ordering::Relaxed));
    let url = WebviewUrl::App(format!("index.html?boot={boot}").into());
    let mut builder = WebviewWindowBuilder::new(&app, label.clone(), url)
        .title("SeekerMail")
        // Open at the same comfortable size as the main window so the detached view shows in
        // full. A small default window cramps the shell (it needs ~960px CSS width, and the
        // UI-scale `zoom` inflates that further), clipping the sidebar. Match main: 1280x832.
        .inner_size(1280.0, 832.0)
        .min_inner_size(960.0, 600.0);
    // The Overlay title bar floats over our content so the parchment drag strip shows through
    // behind the traffic lights (Transparent would show the white window bg instead), matching
    // the main window's top strip. `title_bar_style`/`hidden_title` are macOS-only builder
    // methods — they don't exist on the Windows/Linux builders — so gate them by target_os to
    // keep the cross-platform build compiling (the Windows CI leg fails E0599 otherwise).
    #[cfg(target_os = "macos")]
    {
        builder = builder
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true);
    }
    if let Some(p) = at {
        builder = builder.position(p.x, p.y);
    } else {
        // No explicit drop position (menu/detach) → center on screen rather than Tauri's
        // top-left default, so the new window appears as a complete, well-placed window.
        builder = builder.center();
    }
    builder.build().map_err(internal)?;
    let _ = app.emit(WINDOW_LIST_CHANGED, ());
    Ok(label)
}

/// Close a window by label. Emits the list-changed event.
#[tauri::command]
pub async fn workbench_close_window(app: AppHandle, window_label: String) -> Result<(), IpcError> {
    let win = app
        .get_webview_window(&window_label)
        .ok_or_else(|| IpcError::from(AppError::NotFound))?;
    win.close().map_err(internal)?;
    let _ = app.emit(WINDOW_LIST_CHANGED, ());
    Ok(())
}

/// Focus / raise a window by label (cross-window singleton focus, 18 §9).
#[tauri::command]
pub async fn workbench_focus_window(app: AppHandle, window_label: String) -> Result<(), IpcError> {
    let win = app
        .get_webview_window(&window_label)
        .ok_or_else(|| IpcError::from(AppError::NotFound))?;
    win.set_focus().map_err(internal)?;
    Ok(())
}

/// List the open windows (used to locate a cross-window singleton before opening one).
#[tauri::command]
pub async fn workbench_list_windows(app: AppHandle) -> Result<Vec<WindowInfo>, IpcError> {
    let mut out = Vec::new();
    for (label, win) in app.webview_windows() {
        let focused = win.is_focused().unwrap_or(false);
        out.push(WindowInfo {
            window_label: label,
            focused,
        });
    }
    Ok(out)
}

/// Builder `on_window_event` hook (WB-22): when a detached workbench window is
/// destroyed (OS close button, or the frontend closing the last tab), notify the
/// surviving windows so their window registry refreshes. The main window is
/// ignored — its teardown means the app is quitting.
pub fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if matches!(event, tauri::WindowEvent::Destroyed) && window.label().starts_with("workbench-") {
        let _ = window.app_handle().emit(WINDOW_LIST_CHANGED, ());
    }
}
