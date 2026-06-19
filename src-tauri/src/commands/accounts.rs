//! Account / sync / attachment commands (T013–T026).
//!
//! Thin boundary (03 §1): each command deserializes, calls one service method,
//! and maps `AppError → IpcError`. No business logic lives here. Side-effects on
//! the scheduler (add/remove/trigger) are the one exception — they are one-liners
//! against the managed [`SyncScheduler`].

use std::sync::Arc;

use tauri::State;

use crate::account::{oauth, AccountService};
use crate::error::{AppError, IpcError};
use crate::imap::{attachment, backfill, sampler, SyncScheduler};
use crate::state::AppState;
use crate::storage::{AttachmentRepo, SyncStateRepo};
use crate::types::{
    Account, Attachment, BackfillStatus, CreateAccountParams, DiskUsage, OAuthBeginResult,
    Provider, SamplingResult, SyncState, UpdateAccountParams, VerifyConnectionParams,
    VerifyConnectionResult,
};

// ── Account CRUD (T013) ──────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_accounts(state: State<'_, AppState>) -> Result<Vec<Account>, IpcError> {
    AccountService::list(&state).await.map_err(IpcError::from)
}

#[tauri::command]
pub async fn get_account(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Account, IpcError> {
    AccountService::get(&state, &account_id)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub async fn create_account(
    state: State<'_, AppState>,
    scheduler: State<'_, Arc<SyncScheduler>>,
    params: CreateAccountParams,
) -> Result<Account, IpcError> {
    let account = AccountService::create(&state, params)
        .await
        .map_err(IpcError::from)?;
    scheduler.add_account(&account.id);
    Ok(account)
}

#[tauri::command]
pub async fn update_account(
    state: State<'_, AppState>,
    account_id: String,
    patch: UpdateAccountParams,
) -> Result<Account, IpcError> {
    AccountService::update(&state, &account_id, patch)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub async fn delete_account(
    state: State<'_, AppState>,
    scheduler: State<'_, Arc<SyncScheduler>>,
    account_id: String,
) -> Result<(), IpcError> {
    AccountService::delete(&state, &account_id)
        .await
        .map_err(IpcError::from)?;
    scheduler.remove_account(&account_id);
    Ok(())
}

#[tauri::command]
pub async fn update_account_password(
    state: State<'_, AppState>,
    account_id: String,
    password: String,
) -> Result<(), IpcError> {
    AccountService::update_password(&state, &account_id, password)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub async fn enable_account(
    state: State<'_, AppState>,
    scheduler: State<'_, Arc<SyncScheduler>>,
    account_id: String,
) -> Result<Account, IpcError> {
    let account = AccountService::set_active(&state, &account_id, true)
        .await
        .map_err(IpcError::from)?;
    scheduler.add_account(&account.id);
    Ok(account)
}

#[tauri::command]
pub async fn disable_account(
    state: State<'_, AppState>,
    scheduler: State<'_, Arc<SyncScheduler>>,
    account_id: String,
) -> Result<Account, IpcError> {
    let account = AccountService::set_active(&state, &account_id, false)
        .await
        .map_err(IpcError::from)?;
    scheduler.remove_account(&account_id);
    Ok(account)
}

/// Promote an account to primary (T091). Enforces the single-primary invariant
/// atomically; `NOT_FOUND` if the account does not exist or is inactive.
#[tauri::command]
pub async fn set_primary_account(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Account, IpcError> {
    AccountService::set_primary(&state, &account_id)
        .await
        .map_err(IpcError::from)
}

// ── Connection probe (T014) ──────────────────────────────────────────────────

#[tauri::command]
pub async fn verify_account_connection(
    state: State<'_, AppState>,
    params: VerifyConnectionParams,
) -> Result<VerifyConnectionResult, IpcError> {
    // In-band result — Ok even when the probe fails (02 §verify_account_connection).
    AccountService::verify_connection(&state, params)
        .await
        .map_err(IpcError::from)
}

// ── OAuth (T015) ─────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn begin_oauth_flow(
    state: State<'_, AppState>,
    provider: Provider,
    account_id: String,
) -> Result<OAuthBeginResult, IpcError> {
    let (url, state_nonce) = oauth::begin(&state, provider, &account_id).map_err(IpcError::from)?;
    open_url(&url);
    Ok(OAuthBeginResult {
        authorize_url: url,
        state: state_nonce,
    })
}

#[tauri::command]
pub async fn complete_oauth_flow(
    state: State<'_, AppState>,
    scheduler: State<'_, Arc<SyncScheduler>>,
    code: String,
    state_nonce: String,
) -> Result<(), IpcError> {
    let (account_id, _expiry) = oauth::complete(&state, &code, &state_nonce)
        .await
        .map_err(IpcError::from)?;
    // Token stored — clear any prior auth-error and (re)start polling so mail
    // imports immediately, even if the account's first poll already failed and
    // its task exited (the T021 poll loop stops on an auth error).
    let _ = crate::storage::AccountRepo::new(state.storage.db())
        .clear_auth_error(&account_id)
        .await;
    scheduler.add_account(&account_id);
    scheduler.trigger_now(&account_id);
    Ok(())
}

#[tauri::command]
pub async fn reauth_account(
    state: State<'_, AppState>,
    scheduler: State<'_, Arc<SyncScheduler>>,
    account_id: String,
    password: Option<String>,
) -> Result<(), IpcError> {
    // IMAP: store the new password + clear the auth-error so polling resumes.
    if let Some(pw) = password {
        AccountService::update_password(&state, &account_id, pw)
            .await
            .map_err(IpcError::from)?;
        crate::storage::AccountRepo::new(state.storage.db())
            .clear_auth_error(&account_id)
            .await
            .map_err(IpcError::from)?;
        scheduler.add_account(&account_id);
    }
    Ok(())
}

// ── Knowledge depth + sampling (T016) ────────────────────────────────────────

#[tauri::command]
pub async fn sample_mailbox(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<SamplingResult, IpcError> {
    sampler::sample_mailbox(&state, &account_id)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub async fn set_knowledge_depth(
    state: State<'_, AppState>,
    account_id: String,
    months: Option<u32>,
) -> Result<Account, IpcError> {
    let account = AccountService::set_knowledge_depth(&state, &account_id, months)
        .await
        .map_err(IpcError::from)?;
    // Kick off (or restart) the history backfill for the new depth.
    backfill::spawn_start((*state).clone(), account_id);
    Ok(account)
}

// ── Disk usage (T020) ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_account_disk_usage(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<DiskUsage, IpcError> {
    state
        .storage
        .blobs()
        .account_disk_usage(&account_id, state.storage.db())
        .await
        .map_err(IpcError::from)
}

// ── Sync control + state (T021) ──────────────────────────────────────────────

#[tauri::command]
pub async fn trigger_sync(
    scheduler: State<'_, Arc<SyncScheduler>>,
    account_id: String,
) -> Result<(), IpcError> {
    scheduler.trigger_now(&account_id);
    Ok(())
}

#[tauri::command]
pub async fn get_sync_state(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<SyncState, IpcError> {
    SyncStateRepo::new(state.storage.db())
        .get(&account_id)
        .await
        .map_err(IpcError::from)
}

// ── Backfill control (T022) ──────────────────────────────────────────────────

#[tauri::command]
pub async fn get_backfill_status(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<BackfillStatus, IpcError> {
    crate::storage::BackfillRepo::new(state.storage.db())
        .get(&account_id)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub async fn pause_backfill(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<(), IpcError> {
    backfill::pause(&state, &account_id)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub async fn resume_backfill(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<(), IpcError> {
    backfill::resume((*state).clone(), account_id)
        .await
        .map_err(IpcError::from)
}

// ── Attachments (T025/T026) ──────────────────────────────────────────────────

#[tauri::command]
pub async fn download_attachment(
    state: State<'_, AppState>,
    attachment_id: String,
) -> Result<String, IpcError> {
    attachment::download_one(&state, &attachment_id, attachment::DownloadMode::Manual)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub async fn get_attachments_for_mail(
    state: State<'_, AppState>,
    mail_id: String,
) -> Result<Vec<Attachment>, IpcError> {
    AttachmentRepo::new(state.storage.db())
        .list_by_mail(&mail_id)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub async fn open_attachment(
    state: State<'_, AppState>,
    attachment_id: String,
) -> Result<(), IpcError> {
    attachment::open_attachment(&state, &attachment_id)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub async fn reveal_attachment(
    state: State<'_, AppState>,
    attachment_id: String,
) -> Result<(), IpcError> {
    attachment::reveal_attachment(&state, &attachment_id)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub async fn get_attachment_local_path(
    state: State<'_, AppState>,
    attachment_id: String,
) -> Result<Option<String>, IpcError> {
    attachment::get_local_path(&state, &attachment_id)
        .await
        .map_err(IpcError::from)
}

/// Open a URL in the system browser. (Production should adopt
/// `tauri-plugin-opener`; this keeps the crate plugin-free for now.)
fn open_url(url: &str) {
    let _ = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    }
    .map_err(|e| AppError::FsPermission(format!("open url: {e}")));
}
