//! `IdentityStore` trait — the abstract persistence layer for the cloud service.
//!
//! The only production implementation is `PgIdentityStore` (Postgres via sqlx).
//! In tests you can supply any struct that implements this trait.
//!
//! Redline: this store NEVER holds mail bodies, attachments, contacts, or GTE
//! vectors.  It only knows: who the user is, what plan they are on, what
//! devices have active sessions, and whether they opted into marketing email.

pub mod postgres;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppResult;

// ── Types ────────────────────────────────────────────────────────────────────

/// A registered user identity (server-side mirror of the client `seekermail_id` row).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub id: Uuid,
    pub provider: String,         // always "google" for now
    pub provider_subject: String, // Google "sub" claim
    pub email: String,
    pub email_verified: bool,
    pub display_name: Option<String>,
    pub plan: String, // "free" | "pro"
    pub marketing_consent: bool,
    pub marketing_consent_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating or updating an identity (from a verified OIDC id_token).
#[derive(Debug, Clone)]
pub struct UpsertIdentity {
    pub provider: String,
    pub provider_subject: String,
    pub email: String,
    pub email_verified: bool,
    pub display_name: Option<String>,
}

/// An active session (bearer token → identity mapping).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub identity_id: Uuid,
    pub device_name: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

// ── Trait ────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait IdentityStore: Send + Sync + 'static {
    /// Create or update the identity row after a successful OIDC verification.
    /// Uses `(provider, provider_subject)` as the natural key.
    async fn upsert_identity(&self, input: &UpsertIdentity) -> AppResult<Identity>;

    /// Look up an identity by its UUID.
    async fn get_identity(&self, id: Uuid) -> AppResult<Option<Identity>>;

    /// Create a new session token for the given identity.
    /// `device_name` is a human-readable label (e.g. "MacBook Pro").
    async fn create_session(
        &self,
        identity_id: Uuid,
        device_name: &str,
        ttl_secs: i64,
    ) -> AppResult<Session>;

    /// Validate a bearer token and return the active session, or `None` if
    /// the token is missing, expired, or revoked.
    async fn validate_session(&self, token: &str) -> AppResult<Option<Session>>;

    /// Revoke (delete) a session — called on sign-out.
    async fn revoke_session(&self, token: &str) -> AppResult<()>;

    /// Update the marketing-consent flag for an identity.
    async fn set_marketing_consent(&self, identity_id: Uuid, consent: bool) -> AppResult<()>;

    /// Delete all expired sessions (call this periodically from a background task).
    async fn purge_expired_sessions(&self) -> AppResult<u64>;
}
