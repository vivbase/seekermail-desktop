//! Settings commands (T050/T051) — the `app_settings` KV surface (02 §settings).
//!
//! `get_setting` / `set_setting` move raw JSON strings; the frontend hook layer
//! (`src/ipc/queries/settings.ts`) owns (de)serialisation. Keys are validated
//! against an allow-list of namespaces so arbitrary rows can't be written from
//! the webview. `apply_privacy_policy` (T051) validates the two privacy enums,
//! persists them, and logs a content-free `privacy_policy_changed` event.

use tauri::State;

use crate::error::{AppError, AppResult, IpcError};
use crate::state::AppState;
use crate::storage::settings_repo::SettingRepo;
use crate::types::{ImagePolicy, TrackerPolicy};

/// Key namespaces the webview may read/write. Anything else is a validation
/// error — settings rows owned by backend modules stay backend-owned.
const ALLOWED_KEY_PREFIXES: &[&str] = &["ui.", "privacy.", "gte."];

/// `app_settings` keys for the two T051 privacy policies.
pub const TRACKER_POLICY_KEY: &str = "privacy.tracker_policy";
pub const REMOTE_IMAGE_POLICY_KEY: &str = "privacy.remote_image_policy";
/// `app_settings` key for the T050 theme preference.
pub const THEME_KEY: &str = "ui.theme";
/// `app_settings` key for the analysis-25 UI scale (text size) preference.
pub const FONT_SCALE_KEY: &str = "ui.font_scale";

fn validate_key(key: &str) -> AppResult<()> {
    if ALLOWED_KEY_PREFIXES.iter().any(|p| key.starts_with(p)) {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "setting key namespace not allowed: {key}"
        )))
    }
}

async fn do_get_setting(state: &AppState, key: &str) -> AppResult<Option<String>> {
    validate_key(key)?;
    SettingRepo::new(state.storage.db()).get(key).await
}

async fn do_set_setting(state: &AppState, key: &str, value: &str) -> AppResult<()> {
    validate_key(key)?;
    // Reject values that are not valid JSON so the KV store stays parseable.
    serde_json::from_str::<serde_json::Value>(value)
        .map_err(|_| AppError::Validation(format!("setting value is not valid JSON: {key}")))?;
    SettingRepo::new(state.storage.db()).set(key, value).await?;
    // Key only — values may eventually carry user data; the key never does (09 §5).
    tracing::info!(event = "setting_changed", key = key, "app setting updated");
    Ok(())
}

async fn do_apply_privacy_policy(
    state: &AppState,
    tracker_policy: TrackerPolicy,
    remote_image_policy: ImagePolicy,
) -> AppResult<()> {
    let repo = SettingRepo::new(state.storage.db());
    repo.set(
        TRACKER_POLICY_KEY,
        &serde_json::to_string(&tracker_policy).expect("enum serialises"),
    )
    .await?;
    repo.set(
        REMOTE_IMAGE_POLICY_KEY,
        &serde_json::to_string(&remote_image_policy).expect("enum serialises"),
    )
    .await?;
    // Content-free by construction: two enum tags, no mail data (09 §5).
    tracing::info!(
        event = "privacy_policy_changed",
        tracker_policy = tracker_policy.as_wire(),
        image_policy = remote_image_policy.as_wire(),
        "privacy policy applied"
    );
    Ok(())
}

/// Seed the privacy defaults on first run so tracking protection is ON by
/// default (F_B2 §4.4). Called from the app `setup` hook; existing values are
/// never overwritten.
pub async fn ensure_privacy_defaults(state: &AppState) -> AppResult<()> {
    let repo = SettingRepo::new(state.storage.db());
    if repo.get(TRACKER_POLICY_KEY).await?.is_none() {
        repo.set(
            TRACKER_POLICY_KEY,
            &serde_json::to_string(&TrackerPolicy::BlockKnown).expect("enum serialises"),
        )
        .await?;
    }
    if repo.get(REMOTE_IMAGE_POLICY_KEY).await?.is_none() {
        repo.set(
            REMOTE_IMAGE_POLICY_KEY,
            &serde_json::to_string(&ImagePolicy::BlockAll).expect("enum serialises"),
        )
        .await?;
    }
    Ok(())
}

/// Boot-time theme read for the FOUC guard (T050 §6). Returns one of
/// `"light"` / `"dark"` / `"system"`; anything missing or malformed falls back
/// to `"system"`.
pub async fn initial_theme(state: &AppState) -> String {
    let raw = SettingRepo::new(state.storage.db())
        .get(THEME_KEY)
        .await
        .ok()
        .flatten();
    match raw
        .and_then(|v| serde_json::from_str::<String>(&v).ok())
        .as_deref()
    {
        Some("light") => "light".to_string(),
        Some("dark") => "dark".to_string(),
        _ => "system".to_string(),
    }
}

/// Boot-time UI scale read for the FOUC guard (analysis 25). Returns a clamped
/// multiplier in `[0.9, 1.5]`; anything missing or malformed falls back to `1.0`
/// so the UI renders at 100%.
pub async fn initial_font_scale(state: &AppState) -> f64 {
    const MIN: f64 = 0.9;
    const MAX: f64 = 1.5;
    let raw = SettingRepo::new(state.storage.db())
        .get(FONT_SCALE_KEY)
        .await
        .ok()
        .flatten();
    match raw.and_then(|v| serde_json::from_str::<f64>(&v).ok()) {
        Some(n) if n.is_finite() => n.clamp(MIN, MAX),
        _ => 1.0,
    }
}

/// Raw JSON value for an allow-listed settings key, or `null` when unset.
#[tauri::command]
pub async fn get_setting(
    state: State<'_, AppState>,
    key: String,
) -> Result<Option<String>, IpcError> {
    do_get_setting(&state, &key).await.map_err(IpcError::from)
}

/// Upsert a JSON value for an allow-listed settings key.
#[tauri::command]
pub async fn set_setting(
    state: State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), IpcError> {
    do_set_setting(&state, &key, &value)
        .await
        .map_err(IpcError::from)
}

/// Persist both privacy policies and notify the B1/B2 pipeline (T051). The
/// sanitize/tracker modules read `app_settings` per mail-processing pass, so a
/// DB write is the hot-update path (T051 §6 DB-deferred mode).
#[tauri::command]
pub async fn apply_privacy_policy(
    state: State<'_, AppState>,
    tracker_policy: TrackerPolicy,
    remote_image_policy: ImagePolicy,
) -> Result<(), IpcError> {
    do_apply_privacy_policy(&state, tracker_policy, remote_image_policy)
        .await
        .map_err(IpcError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ErrorCode;

    #[tokio::test]
    async fn get_set_roundtrip_for_allowed_key() {
        let (state, _rx) = AppState::test_state().await;
        // `ui.theme` ships seeded to the default by migration 001_init.
        assert_eq!(
            do_get_setting(&state, "ui.theme").await.unwrap().as_deref(),
            Some("\"system\"")
        );
        do_set_setting(&state, "ui.theme", "\"dark\"")
            .await
            .unwrap();
        assert_eq!(
            do_get_setting(&state, "ui.theme").await.unwrap().as_deref(),
            Some("\"dark\"")
        );
    }

    #[tokio::test]
    async fn disallowed_key_namespace_is_validation_error() {
        let (state, _rx) = AppState::test_state().await;
        let err = do_set_setting(&state, "imap.password", "\"x\"")
            .await
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::Validation);
        let err = do_get_setting(&state, "secrets.token").await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::Validation);
    }

    #[tokio::test]
    async fn non_json_value_is_validation_error() {
        let (state, _rx) = AppState::test_state().await;
        let err = do_set_setting(&state, "ui.theme", "dark — not json")
            .await
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::Validation);
    }

    #[tokio::test]
    async fn apply_privacy_policy_persists_both_keys() {
        let (state, _rx) = AppState::test_state().await;
        do_apply_privacy_policy(&state, TrackerPolicy::BlockAll, ImagePolicy::AllowAll)
            .await
            .unwrap();
        assert_eq!(
            do_get_setting(&state, TRACKER_POLICY_KEY)
                .await
                .unwrap()
                .as_deref(),
            Some("\"block_all\"")
        );
        assert_eq!(
            do_get_setting(&state, REMOTE_IMAGE_POLICY_KEY)
                .await
                .unwrap()
                .as_deref(),
            Some("\"allow_all\"")
        );
    }

    #[tokio::test]
    async fn defaults_seed_once_and_never_overwrite() {
        let (state, _rx) = AppState::test_state().await;
        ensure_privacy_defaults(&state).await.unwrap();
        assert_eq!(
            do_get_setting(&state, TRACKER_POLICY_KEY)
                .await
                .unwrap()
                .as_deref(),
            Some("\"block_known\"")
        );
        assert_eq!(
            do_get_setting(&state, REMOTE_IMAGE_POLICY_KEY)
                .await
                .unwrap()
                .as_deref(),
            Some("\"block_all\"")
        );
        // A user choice survives a second seed pass.
        do_set_setting(&state, TRACKER_POLICY_KEY, "\"allow_all\"")
            .await
            .unwrap();
        ensure_privacy_defaults(&state).await.unwrap();
        assert_eq!(
            do_get_setting(&state, TRACKER_POLICY_KEY)
                .await
                .unwrap()
                .as_deref(),
            Some("\"allow_all\"")
        );
    }

    #[tokio::test]
    async fn initial_theme_falls_back_to_system() {
        let (state, _rx) = AppState::test_state().await;
        assert_eq!(initial_theme(&state).await, "system");
        do_set_setting(&state, THEME_KEY, "\"dark\"").await.unwrap();
        assert_eq!(initial_theme(&state).await, "dark");
        do_set_setting(&state, THEME_KEY, "\"purple\"")
            .await
            .unwrap();
        assert_eq!(initial_theme(&state).await, "system");
    }

    #[tokio::test]
    async fn initial_font_scale_clamps_and_falls_back() {
        let (state, _rx) = AppState::test_state().await;
        // Unset → default 1.0.
        assert_eq!(initial_font_scale(&state).await, 1.0);
        // In-range value is returned as-is.
        do_set_setting(&state, FONT_SCALE_KEY, "1.15")
            .await
            .unwrap();
        assert_eq!(initial_font_scale(&state).await, 1.15);
        // Out-of-range is clamped to the ceiling.
        do_set_setting(&state, FONT_SCALE_KEY, "9").await.unwrap();
        assert_eq!(initial_font_scale(&state).await, 1.5);
        // Malformed (JSON string, not a number) → default 1.0.
        do_set_setting(&state, FONT_SCALE_KEY, "\"big\"")
            .await
            .unwrap();
        assert_eq!(initial_font_scale(&state).await, 1.0);
    }
}
