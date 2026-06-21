//! POST /v1/session  — exchange a Google id_token for a SeekerMail session token.
//! DELETE /v1/session — revoke the current session (sign out).

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::{
    error::{AppError, AppResult},
    routes::{extract_bearer, AppState},
    store::{IdentityStore, UpsertIdentity},
};

// ── Request / Response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    /// A Google OIDC id_token (from `complete_google_signin` on the desktop client).
    pub id_token: String,
    /// Human-readable name for this device (optional, defaults to "desktop").
    #[serde(default = "default_device_name")]
    pub device_name: String,
}

fn default_device_name() -> String {
    "desktop".to_string()
}

#[derive(Debug, Serialize)]
pub struct CreateSessionResponse {
    /// Opaque bearer token — store this securely on the client.
    pub token: String,
    /// The identity the token belongs to.
    pub identity: crate::store::Identity,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// Exchange a Google id_token for a SeekerMail cloud session token.
///
/// Flow:
///   1. Verify the id_token with Google's JWKS (scope guard: openid email profile only).
///   2. Upsert the identity row (create on first login, refresh on subsequent logins).
///   3. Issue a new session token.
///   4. Return the token + identity to the desktop client.
pub async fn create_session<S: IdentityStore>(
    State(state): State<AppState<S>>,
    Json(body): Json<CreateSessionRequest>,
) -> AppResult<(StatusCode, Json<CreateSessionResponse>)> {
    // Step 1: verify the id_token.
    let claims = state.oidc.verify(&body.id_token).await?;

    // Step 2: upsert the identity.
    let input = UpsertIdentity {
        provider: "google".to_string(),
        provider_subject: claims.sub,
        email: claims.email,
        email_verified: claims.email_verified,
        display_name: claims.name,
    };
    let identity = state.store.upsert_identity(&input).await?;

    // Step 3: create a session.
    let session = state
        .store
        .create_session(
            identity.id,
            &body.device_name,
            state.config.session_ttl_secs,
        )
        .await?;

    tracing::info!(
        identity_id = %identity.id,
        email = %identity.email,
        "session created"
    );

    Ok((
        StatusCode::CREATED,
        Json(CreateSessionResponse {
            token: session.token,
            identity,
        }),
    ))
}

/// Revoke the current session (sign out).
/// Requires `Authorization: Bearer <token>` header.
pub async fn revoke_session<S: IdentityStore>(
    State(state): State<AppState<S>>,
    headers: HeaderMap,
) -> AppResult<StatusCode> {
    let token = extract_bearer(&headers).ok_or(AppError::Unauthorized)?;
    state.store.revoke_session(token).await?;
    tracing::info!("session revoked");
    Ok(StatusCode::NO_CONTENT)
}
