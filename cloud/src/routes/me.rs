//! GET /v1/me          — return the identity for the current session.
//! POST /v1/me/marketing-consent — update marketing consent flag.

use axum::{extract::State, http::HeaderMap, Json};
use serde::{Deserialize, Serialize};

use crate::{
    error::{AppError, AppResult},
    routes::{extract_bearer, AppState},
    store::{Identity, IdentityStore},
};

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub identity: Identity,
}

#[derive(Debug, Deserialize)]
pub struct SetMarketingConsentRequest {
    pub consent: bool,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// Return the identity for the current bearer token.
pub async fn get_me<S: IdentityStore>(
    State(state): State<AppState<S>>,
    headers: HeaderMap,
) -> AppResult<Json<MeResponse>> {
    let token = extract_bearer(&headers).ok_or(AppError::Unauthorized)?;

    let session = state
        .store
        .validate_session(token)
        .await?
        .ok_or(AppError::SessionNotFound)?;

    let identity = state
        .store
        .get_identity(session.identity_id)
        .await?
        .ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("identity not found for valid session"))
        })?;

    Ok(Json(MeResponse { identity }))
}

/// Update the marketing-consent flag for the signed-in user.
/// Requires `Authorization: Bearer <token>` header.
pub async fn set_marketing_consent<S: IdentityStore>(
    State(state): State<AppState<S>>,
    headers: HeaderMap,
    Json(body): Json<SetMarketingConsentRequest>,
) -> AppResult<Json<MeResponse>> {
    let token = extract_bearer(&headers).ok_or(AppError::Unauthorized)?;

    let session = state
        .store
        .validate_session(token)
        .await?
        .ok_or(AppError::SessionNotFound)?;

    state
        .store
        .set_marketing_consent(session.identity_id, body.consent)
        .await?;

    let identity = state
        .store
        .get_identity(session.identity_id)
        .await?
        .ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("identity not found after consent update"))
        })?;

    Ok(Json(MeResponse { identity }))
}
