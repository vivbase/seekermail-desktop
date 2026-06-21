use std::sync::Arc;

use axum::{
    http::{header, HeaderMap, StatusCode},
    routing::{delete, get, post},
    Json, Router,
};
use serde_json::json;

use crate::{config::Config, oidc::OidcVerifier, store::IdentityStore};

pub mod me;
pub mod session;

// ── AppState ──────────────────────────────────────────────────────────────────

/// Shared application state injected into every route handler.
/// Manual Clone so the derive macro does not add an unnecessary `S: Clone` bound
/// (all fields are Arc<...>, which clone by ref-count regardless of S).
pub struct AppState<S: IdentityStore> {
    pub store: Arc<S>,
    pub oidc: Arc<OidcVerifier>,
    pub config: Arc<Config>,
}

impl<S: IdentityStore> Clone for AppState<S> {
    fn clone(&self) -> Self {
        AppState {
            store: Arc::clone(&self.store),
            oidc: Arc::clone(&self.oidc),
            config: Arc::clone(&self.config),
        }
    }
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn build_router<S: IdentityStore>(state: AppState<S>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/session", post(session::create_session::<S>))
        .route("/v1/session", delete(session::revoke_session::<S>))
        .route("/v1/me", get(me::get_me::<S>))
        .route(
            "/v1/me/marketing-consent",
            post(me::set_marketing_consent::<S>),
        )
        .with_state(state)
}

// ── Health check ─────────────────────────────────────────────────────────────

async fn health() -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}

// ── Bearer token extractor ────────────────────────────────────────────────────

/// Extract a bearer token from the `Authorization: Bearer <token>` header.
pub fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}
