//! Mail reading-view commands — tracker info + remote-image allow (T029) — plus
//! compose send + cancel (T043).

use tauri::State;

use crate::error::IpcError;
use crate::send;
use crate::state::AppState;
use crate::storage::{MailRepo, SettingRepo};
use crate::types::{
    CancelSendResult, ImageAllowScope, ListMailsParams, ListThreadsParams, MailDetail, MailSummary,
    PageResult, SendMailParams, SendMailResult, Thread, TrackerInfo,
};

/// Read tracker status for one mail + whether its sender's images are allowed.
#[tauri::command]
pub async fn get_tracker_info(
    state: State<'_, AppState>,
    mail_id: String,
) -> Result<TrackerInfo, IpcError> {
    let (sender_email, blocked, tracker_count) = MailRepo::new(state.storage.db())
        .tracker_row(&mail_id)
        .await
        .map_err(IpcError::from)?;
    let images_allowed = SettingRepo::new(state.storage.db())
        .is_sender_image_allowed(&sender_email)
        .await
        .map_err(IpcError::from)?;
    Ok(TrackerInfo {
        blocked,
        tracker_count,
        images_allowed,
        sender_email,
    })
}

/// Persist a remote-image allow decision. `ThisMessage` is handled entirely on the
/// frontend (a one-shot DOM swap); only `AlwaysSender` persists (T029 §3).
#[tauri::command]
pub async fn allow_remote_images(
    state: State<'_, AppState>,
    mail_id: String,
    scope: ImageAllowScope,
) -> Result<(), IpcError> {
    match scope {
        ImageAllowScope::ThisMessage => {
            tracing::debug!(mail_id = %mail_id, "remote images allowed for this message (frontend DOM)");
        }
        ImageAllowScope::AlwaysSender { sender_email } => {
            SettingRepo::new(state.storage.db())
                .add_image_allow_sender(&sender_email)
                .await
                .map_err(IpcError::from)?;
        }
    }
    Ok(())
}

/// Queue a message for send behind the 10-second cancel window (T043). Returns
/// immediately with the pending id (for `cancel_send`) and the message id.
#[tauri::command]
pub async fn send_mail(
    state: State<'_, AppState>,
    params: SendMailParams,
) -> Result<SendMailResult, IpcError> {
    send::schedule_send(&state, params)
        .await
        .map_err(IpcError::from)
}

/// Cancel a pending send within its window. Two-path lookup (T043 + T085 §6):
/// `pendingId` is first tried against the in-memory SMTP queue (a T043
/// `schedule_send` pending id, 10 s window); when absent there it is treated
/// as an E3 `ai_drafts.id` inside its 30 s `send_after` window — a successful
/// E3 cancel clears `send_after`, keeps the draft `pending`, audits
/// `auto_send_cancelled`, and re-emits `draft:ready` so the draft returns to
/// the Pending review queue. `cancelled=false` if both windows have elapsed
/// or the id is unknown.
#[tauri::command]
pub async fn cancel_send(
    state: State<'_, AppState>,
    pending_id: String,
) -> Result<CancelSendResult, IpcError> {
    let direct = send::cancel_send(&state, &pending_id);
    if direct.cancelled {
        return Ok(direct);
    }
    crate::ai::pipeline::e3_send_queue::cancel_pending_auto_send(&state, &pending_id)
        .await
        .map_err(IpcError::from)
}

// ── Mail-list read backend (G2/G3) ───────────────────────────────────────────
// The L0 stream (`ThreadList`) and the reading view consume these. Without them
// the webview's `invoke("list_threads" | "list_mails" | "get_mail")` calls reject
// and every mail surface renders empty.

/// Paginated thread list for the folded L0 stream (G2).
#[tauri::command]
pub async fn list_threads(
    state: State<'_, AppState>,
    params: ListThreadsParams,
) -> Result<PageResult<Thread>, IpcError> {
    MailRepo::new(state.storage.db())
        .list_threads(&params)
        .await
        .map_err(IpcError::from)
}

/// Paginated flat mail list — unread / processed / all-mail routes (G3).
#[tauri::command]
pub async fn list_mails(
    state: State<'_, AppState>,
    params: ListMailsParams,
) -> Result<PageResult<MailSummary>, IpcError> {
    MailRepo::new(state.storage.db())
        .list_mails(&params)
        .await
        .map_err(IpcError::from)
}

/// Full mail detail for the reading view (G3).
#[tauri::command]
pub async fn get_mail(state: State<'_, AppState>, mail_id: String) -> Result<MailDetail, IpcError> {
    MailRepo::new(state.storage.db())
        .get_mail(&mail_id)
        .await
        .map_err(IpcError::from)
}

/// Mark a mail read/unread (drives `mail:updated`).
#[tauri::command]
pub async fn set_mail_read(
    state: State<'_, AppState>,
    mail_id: String,
    is_read: bool,
) -> Result<(), IpcError> {
    MailRepo::new(state.storage.db())
        .set_read(&mail_id, is_read)
        .await
        .map_err(IpcError::from)
}

/// Star / unstar a mail.
#[tauri::command]
pub async fn set_mail_starred(
    state: State<'_, AppState>,
    mail_id: String,
    is_starred: bool,
) -> Result<(), IpcError> {
    MailRepo::new(state.storage.db())
        .set_starred(&mail_id, is_starred)
        .await
        .map_err(IpcError::from)
}

/// Archive a mail (removes it from the active streams).
#[tauri::command]
pub async fn archive_mail(state: State<'_, AppState>, mail_id: String) -> Result<(), IpcError> {
    MailRepo::new(state.storage.db())
        .set_archived(&mail_id, true)
        .await
        .map_err(IpcError::from)
}

/// Soft-delete a mail (moves it to Trash; `is_deleted = 1`).
#[tauri::command]
pub async fn delete_mail(state: State<'_, AppState>, mail_id: String) -> Result<(), IpcError> {
    MailRepo::new(state.storage.db())
        .set_deleted(&mail_id, true)
        .await
        .map_err(IpcError::from)
}
