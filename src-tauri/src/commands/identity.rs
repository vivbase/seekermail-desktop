//! SeekerMail ID identity commands (A6, decoupled model).
//!
//! The SeekerMail ID is INDEPENDENT of imported mailboxes: it is created by signing
//! in with Google (OIDC) and is OPTIONAL — the app is fully usable with no identity
//! (local-first). These commands own the local identity row (get / sign-out /
//! marketing-consent) and the Google sign-in entry points.
//!
//! Thin boundary (03 §1): each command deserializes, calls one repo/service method,
//! and maps `AppError → IpcError`.
//!
//! Spec: knowledge base `docs/function list/F_A6_seekermail_id.md` (rewritten —
//! binding-mailbox model removed) and `docs/analysis/26_identity_decoupling_and_
//! email_marketing_foundation.md`.
//!
//! Sign-in status: Layer 1 (client-side Google OIDC) is implemented here via
//! `crate::identity` — PKCE + a loopback redirect (`http://127.0.0.1:<port>`,
//! analysis/27) + `id_token` JWKS verification, scopes `openid email profile`
//! ONLY. It needs the `SEEKERMAIL_GOOGLE_CLIENT_ID` env var (docs/dev/15) and no
//! SeekerMail server. Cloud entitlement / marketing sync remain the separate
//! Layer 2 backend (still T121). With the client id unset, `begin_google_signin`
//! returns `AUTH_OAUTH_FAILED` — exactly the mailbox-OAuth "not configured" path.

use tauri::State;

use crate::error::IpcError;
use crate::state::AppState;
use crate::storage::IdentityRepo;
use crate::types::{OAuthBeginResult, SeekerMailId};

/// The current SeekerMail ID, or `null` when signed out (the local-first default).
#[tauri::command]
pub async fn get_seekermail_id(
    state: State<'_, AppState>,
) -> Result<Option<SeekerMailId>, IpcError> {
    IdentityRepo::new(state.storage.db())
        .get()
        .await
        .map_err(IpcError::from)
}

/// Sign out of the SeekerMail ID (A6, decoupled model). Clears ONLY the local
/// identity row — mailboxes, local mail, and the GTE index are untouched. This
/// replaces the old behaviour where signing out disconnected every mailbox; in the
/// decoupled model identity and mailboxes are independent. The authoritative
/// server-side session revoke lands with the cloud-identity backend (T121).
#[tauri::command]
pub async fn sign_out_seekermail(state: State<'_, AppState>) -> Result<(), IpcError> {
    IdentityRepo::new(state.storage.db())
        .clear()
        .await
        .map_err(IpcError::from)
}

/// Set or withdraw the marketing-consent flag (opt-in; default OFF). Returns the
/// updated identity, or `null` when signed out (nothing to update). Marketing email
/// is first-party only and never implies selling or sharing data (PRIVACY_POLICY).
#[tauri::command]
pub async fn set_marketing_consent(
    state: State<'_, AppState>,
    consent: bool,
    source: Option<String>,
) -> Result<Option<SeekerMailId>, IpcError> {
    IdentityRepo::new(state.storage.db())
        .set_marketing_consent(consent, source)
        .await
        .map_err(IpcError::from)
}

/// Begin "Sign in with Google" for the SeekerMail ID (OIDC, scopes
/// `openid email profile` — NO mail access; mailboxes are connected separately via
/// `begin_oauth_flow`). The Gmail-vs-identity distinction is the §3.5 UX trap in
/// analysis/26. Starts a loopback listener, opens the system browser, and returns
/// the authorization URL + CSRF `state`.
#[tauri::command]
pub async fn begin_google_signin(state: State<'_, AppState>) -> Result<OAuthBeginResult, IpcError> {
    let (url, state_nonce) = crate::identity::begin(&state).map_err(IpcError::from)?;
    open_url(&url);
    Ok(OAuthBeginResult {
        authorize_url: url,
        state: state_nonce,
    })
}

/// Complete "Sign in with Google" (OIDC code → id_token → identity row). With the
/// loopback flow the code is captured by the listener started in
/// `begin_google_signin`, so the frontend passes an empty `code` plus the matching
/// `state_nonce`; a non-empty `code` (manual path) is honored too. Exchanges the
/// PKCE code, verifies the `id_token` against Google's JWKS, and upserts the local
/// identity row via [`IdentityRepo::upsert_signin`] (marketing consent defaults OFF).
#[tauri::command]
pub async fn complete_google_signin(
    state: State<'_, AppState>,
    code: String,
    state_nonce: String,
) -> Result<SeekerMailId, IpcError> {
    crate::identity::complete(&state, &code, &state_nonce)
        .await
        .map_err(IpcError::from)
}

/// Open a URL in the system browser (same approach as `commands::accounts`;
/// production should adopt `tauri-plugin-opener`).
fn open_url(url: &str) {
    let result = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    if let Err(e) = result {
        tracing::warn!(event = "open_url_failed", error = %e, "could not open system browser");
    }
}
