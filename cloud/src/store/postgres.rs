//! `PgIdentityStore` — the production Postgres-backed `IdentityStore`.
//!
//! Uses `sqlx` with compile-time-checked queries where possible, and
//! `query_as!` / `query!` macros for the rest.  The underlying schema is
//! defined in `migrations/001_identity.sql`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rand::RngCore;
use sqlx::PgPool;
use uuid::Uuid;

use super::{Identity, IdentityStore, Session, UpsertIdentity};
use crate::error::{AppError, AppResult};

// ── Helper row types (for sqlx `query_as`) ───────────────────────────────────

#[derive(sqlx::FromRow)]
struct IdentityRow {
    id: Uuid,
    provider: String,
    provider_subject: String,
    email: String,
    email_verified: bool,
    display_name: Option<String>,
    plan: String,
    marketing_consent: bool,
    marketing_consent_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<IdentityRow> for Identity {
    fn from(r: IdentityRow) -> Self {
        Identity {
            id: r.id,
            provider: r.provider,
            provider_subject: r.provider_subject,
            email: r.email,
            email_verified: r.email_verified,
            display_name: r.display_name,
            plan: r.plan,
            marketing_consent: r.marketing_consent,
            marketing_consent_at: r.marketing_consent_at,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    token: String,
    identity_id: Uuid,
    device_name: String,
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

impl From<SessionRow> for Session {
    fn from(r: SessionRow) -> Self {
        Session {
            token: r.token,
            identity_id: r.identity_id,
            device_name: r.device_name,
            expires_at: r.expires_at,
            created_at: r.created_at,
        }
    }
}

// ── PgIdentityStore ───────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct PgIdentityStore {
    pool: PgPool,
}

impl PgIdentityStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Run pending migrations against the connected database.
    pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
        sqlx::migrate!("./migrations").run(pool).await
    }
}

#[async_trait]
impl IdentityStore for PgIdentityStore {
    /// Upsert by (provider, provider_subject).  On conflict we refresh the
    /// mutable fields (email, email_verified, display_name) but preserve
    /// plan and marketing_consent so that paid status is never reset by a
    /// fresh login.
    async fn upsert_identity(&self, input: &UpsertIdentity) -> AppResult<Identity> {
        let row: IdentityRow = sqlx::query_as(
            r#"
            INSERT INTO identities
                (provider, provider_subject, email, email_verified, display_name)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (provider, provider_subject) DO UPDATE SET
                email           = EXCLUDED.email,
                email_verified  = EXCLUDED.email_verified,
                display_name    = EXCLUDED.display_name,
                updated_at      = NOW()
            RETURNING
                id, provider, provider_subject, email, email_verified,
                display_name, plan, marketing_consent, marketing_consent_at,
                created_at, updated_at
            "#,
        )
        .bind(&input.provider)
        .bind(&input.provider_subject)
        .bind(&input.email)
        .bind(input.email_verified)
        .bind(&input.display_name)
        .fetch_one(&self.pool)
        .await
        .map_err(AppError::Db)?;

        Ok(row.into())
    }

    async fn get_identity(&self, id: Uuid) -> AppResult<Option<Identity>> {
        let row: Option<IdentityRow> = sqlx::query_as(
            r#"
            SELECT id, provider, provider_subject, email, email_verified,
                   display_name, plan, marketing_consent, marketing_consent_at,
                   created_at, updated_at
            FROM identities WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Db)?;

        Ok(row.map(Into::into))
    }

    async fn create_session(
        &self,
        identity_id: Uuid,
        device_name: &str,
        ttl_secs: i64,
    ) -> AppResult<Session> {
        // Generate a 32-byte cryptographically random opaque token.
        let mut raw = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut raw);
        let token = hex::encode(raw);

        let expires_at = Utc::now() + chrono::Duration::seconds(ttl_secs);

        let row: SessionRow = sqlx::query_as(
            r#"
            INSERT INTO sessions (token, identity_id, device_name, expires_at)
            VALUES ($1, $2, $3, $4)
            RETURNING token, identity_id, device_name, expires_at, created_at
            "#,
        )
        .bind(&token)
        .bind(identity_id)
        .bind(device_name)
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await
        .map_err(AppError::Db)?;

        Ok(row.into())
    }

    async fn validate_session(&self, token: &str) -> AppResult<Option<Session>> {
        // Fetch the session only if it hasn't expired.
        let row: Option<SessionRow> = sqlx::query_as(
            r#"
            SELECT token, identity_id, device_name, expires_at, created_at
            FROM sessions
            WHERE token = $1 AND expires_at > NOW()
            "#,
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Db)?;

        Ok(row.map(Into::into))
    }

    async fn revoke_session(&self, token: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM sessions WHERE token = $1")
            .bind(token)
            .execute(&self.pool)
            .await
            .map_err(AppError::Db)?;
        Ok(())
    }

    async fn set_marketing_consent(&self, identity_id: Uuid, consent: bool) -> AppResult<()> {
        let consent_at: Option<DateTime<Utc>> = if consent { Some(Utc::now()) } else { None };

        sqlx::query(
            r#"
            UPDATE identities
            SET marketing_consent    = $1,
                marketing_consent_at = $2,
                updated_at           = NOW()
            WHERE id = $3
            "#,
        )
        .bind(consent)
        .bind(consent_at)
        .bind(identity_id)
        .execute(&self.pool)
        .await
        .map_err(AppError::Db)?;

        Ok(())
    }

    async fn purge_expired_sessions(&self) -> AppResult<u64> {
        let result = sqlx::query("DELETE FROM sessions WHERE expires_at <= NOW()")
            .execute(&self.pool)
            .await
            .map_err(AppError::Db)?;
        Ok(result.rows_affected())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    // Integration tests require a live Postgres database.
    // Run with: DATABASE_URL=postgresql://... cargo test --test integration
    // Unit-level smoke tests that don't need a DB go here.

    #[test]
    fn token_is_64_hex_chars() {
        use rand::RngCore;
        let mut raw = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut raw);
        let token = hex::encode(raw);
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
