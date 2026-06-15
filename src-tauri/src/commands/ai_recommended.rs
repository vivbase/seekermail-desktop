//! F3 recommended-provider commands (T064, dev/02 §Module H extension).
//!
//! Thin boundary (03 §1): each command calls one `ai::recommended` service
//! function and maps `AppError → IpcError`.
//!
//! Naming note: the task card lists `begin_oauth_flow` / `complete_oauth_flow`,
//! but those names already belong to the **account-mail** OAuth surface
//! (`commands::accounts`, T015). The recommended-provider grant is a separate
//! flow with separate pending state and a separate deep-link path, so its
//! commands are `begin_recommended_oauth` / `complete_recommended_oauth`.

use tauri::State;

use crate::ai::recommended::{
    self, AiSetupStatus, BeginRecommendedOAuthResult, CompleteRecommendedOAuthResult,
    RecommendedProviderInfo, RecommendedTier,
};
use crate::error::IpcError;
use crate::state::AppState;

/// The built-in recommendation tiers for the wizard (F_F3 §3 step 2). Pure
/// config read — endpoint URLs and client-id env names never cross the wire.
#[tauri::command]
pub async fn get_recommended_providers() -> Result<Vec<RecommendedProviderInfo>, IpcError> {
    Ok(recommended::recommended_provider_infos())
}

/// Wizard/settings snapshot: disclosure confirmation, conservative-quota
/// deadline, first-auth stamp.
#[tauri::command]
pub async fn get_ai_setup_status(state: State<'_, AppState>) -> Result<AiSetupStatus, IpcError> {
    recommended::setup_status(state.storage.db())
        .await
        .map_err(IpcError::from)
}

/// Record the user's data-flow disclosure confirmation (dev/06 §8). The
/// backend `begin_recommended_oauth` gate refuses cloud grants until this has
/// run, which is what makes the modal genuinely non-bypassable.
#[tauri::command]
pub async fn confirm_ai_disclosure(state: State<'_, AppState>) -> Result<AiSetupStatus, IpcError> {
    recommended::confirm_disclosure(state.storage.db())
        .await
        .map_err(IpcError::from)
}

/// Lift the first-week conservative quota early (F_F3 §4.6 — the settings
/// page control).
#[tauri::command]
pub async fn clear_conservative_quota(state: State<'_, AppState>) -> Result<(), IpcError> {
    recommended::clear_conservative_quota(state.storage.db())
        .await
        .map_err(IpcError::from)
}

/// Start a recommended-provider grant: build the PKCE authorize URL, park the
/// CSRF state, and open the **system browser** (never an embedded webview).
/// Errors: `VALIDATION` (unknown tier), `FORBIDDEN` (disclosure not
/// confirmed), `AUTH_OAUTH_FAILED` (client id / endpoint not configured).
#[tauri::command]
pub async fn begin_recommended_oauth(
    state: State<'_, AppState>,
    tier: String,
) -> Result<BeginRecommendedOAuthResult, IpcError> {
    let tier = RecommendedTier::parse(&tier).map_err(IpcError::from)?;
    let result = recommended::begin(&state, tier)
        .await
        .map_err(IpcError::from)?;
    open_url(&result.authorize_url);
    Ok(result)
}

/// Finish a recommended grant from the deep-link callback or the manual
/// code-paste fallback (F_F3 §6). Hard grant failures (state mismatch,
/// expiry, token-endpoint refusal) are `AUTH_OAUTH_FAILED`; a failed
/// connection test is in-band (`ok = false`).
#[tauri::command]
pub async fn complete_recommended_oauth(
    state: State<'_, AppState>,
    state_nonce: String,
    code: String,
) -> Result<CompleteRecommendedOAuthResult, IpcError> {
    recommended::complete(&state, &state_nonce, &code)
        .await
        .map_err(IpcError::from)
}

/// Disconnect a recommended tier (F_F3 §4.5): clear the Keychain token and
/// reset every account using the tier's provider to `ai_provider = 'none'`.
#[tauri::command]
pub async fn revoke_recommended_provider(
    state: State<'_, AppState>,
    tier: String,
) -> Result<(), IpcError> {
    let tier = RecommendedTier::parse(&tier).map_err(IpcError::from)?;
    recommended::revoke(&state, tier)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ErrorCode;

    // The command layer is thin; flow logic is tested in `ai::recommended` and
    // the quota in `ai::registry`. These tests pin the wire-facing behavior.

    #[tokio::test]
    async fn recommendation_list_has_both_tiers() {
        let infos = recommended::recommended_provider_infos();
        assert_eq!(infos.len(), 2);
        let tiers: Vec<&str> = infos.iter().map(|i| i.tier.as_str()).collect();
        assert!(tiers.contains(&"balanced"));
        assert!(tiers.contains(&"high_quality"));
        for info in &infos {
            assert!(info.monthly_cost_min_usd <= info.monthly_cost_max_usd);
            assert!(info.tokens_per_reply_estimate > 0);
        }
    }

    #[tokio::test]
    async fn unknown_tier_is_validation() {
        let err = RecommendedTier::parse("premium").unwrap_err();
        assert_eq!(err.code(), ErrorCode::Validation);
    }

    #[tokio::test]
    async fn disclosure_then_quota_clear_roundtrip() {
        let (state, _rx) = AppState::test_state().await;
        let db = state.storage.db();

        let before = recommended::setup_status(db).await.unwrap();
        assert!(before.disclosure_confirmed_at.is_none());

        let after = recommended::confirm_disclosure(db).await.unwrap();
        assert!(after.disclosure_confirmed_at.is_some());

        // Arm then lift the conservative quota — the settings-page path.
        crate::storage::SettingRepo::new(db)
            .set(
                recommended::CONSERVATIVE_QUOTA_UNTIL_KEY,
                &(crate::util::now_unix() + 86_400).to_string(),
            )
            .await
            .unwrap();
        assert!(recommended::conservative_quota_active(db).await.unwrap());
        recommended::clear_conservative_quota(db).await.unwrap();
        assert!(!recommended::conservative_quota_active(db).await.unwrap());
    }

    #[tokio::test]
    async fn complete_with_bogus_state_is_oauth_failed() {
        let (state, _rx) = AppState::test_state().await;
        let err = recommended::complete(&state, "bogus-nonce", "code")
            .await
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::AuthOauthFailed);
    }
}
