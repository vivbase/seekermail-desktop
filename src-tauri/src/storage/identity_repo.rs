//! `IdentityRepo` — the OPTIONAL SeekerMail ID local cache (A6, decoupled model).
//!
//! At most one row (single-row table, `id = 'self'`). Holds the identity the user
//! signed in with (Google OIDC) plus the OPT-IN marketing-consent flag. It is
//! INDEPENDENT of `accounts` (no FK): mailboxes can be added or removed without
//! touching identity, and signing out clears ONLY this row. Mail bodies, contacts,
//! and GTE vectors never appear here.
//!
//! Spec: knowledge base `docs/function list/F_A6_seekermail_id.md` (rewritten) and
//! `docs/analysis/26_identity_decoupling_and_email_marketing_foundation.md`.

use super::{map_sqlx_err, Db};
use crate::error::AppResult;
use crate::types::SeekerMailId;
use crate::util::now_unix;

/// Stateless repository over the shared pool (mirrors [`super::AccountRepo`]).
#[derive(Clone)]
pub struct IdentityRepo<'a> {
    db: &'a Db,
}

impl<'a> IdentityRepo<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// The current SeekerMail ID, or `None` when signed out (the local-first
    /// default — most installs have no row here).
    pub async fn get(&self) -> AppResult<Option<SeekerMailId>> {
        let row: Option<IdentityRow> = sqlx::query_as(
            "SELECT provider, email, display_name, email_verified, plan, \
             marketing_consent, marketing_consent_source, signed_in_at \
             FROM seekermail_id WHERE id = 'self'",
        )
        .fetch_optional(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(row.map(SeekerMailId::from))
    }

    /// Create/replace the single identity row on a successful sign-in. Carries any
    /// onboarding marketing-consent choice through `consent` / `consent_source`.
    ///
    /// Not yet wired: the real Google OIDC sign-in lands with the cloud-identity
    /// backend (T121). Kept here so that backend only has to call this method.
    #[allow(dead_code)]
    pub async fn upsert_signin(
        &self,
        input: &SignInInput,
        consent: bool,
        consent_source: Option<&str>,
    ) -> AppResult<SeekerMailId> {
        let now = now_unix();
        let consent_at = if consent { Some(now) } else { None };
        sqlx::query(
            "INSERT INTO seekermail_id \
               (id, provider, provider_subject, email, email_verified, display_name, plan, \
                marketing_consent, marketing_consent_source, marketing_consent_at, signed_in_at, \
                created_at, updated_at) \
             VALUES ('self', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               provider = excluded.provider, provider_subject = excluded.provider_subject, \
               email = excluded.email, email_verified = excluded.email_verified, \
               display_name = excluded.display_name, plan = excluded.plan, \
               marketing_consent = excluded.marketing_consent, \
               marketing_consent_source = excluded.marketing_consent_source, \
               marketing_consent_at = excluded.marketing_consent_at, \
               signed_in_at = excluded.signed_in_at, updated_at = excluded.updated_at",
        )
        .bind(&input.provider)
        .bind(&input.provider_subject)
        .bind(&input.email)
        .bind(input.email_verified as i64)
        .bind(&input.display_name)
        .bind(&input.plan)
        .bind(consent as i64)
        .bind(consent_source)
        .bind(consent_at)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(self
            .get()
            .await?
            .expect("identity row exists immediately after upsert"))
    }

    /// Sign out of the SeekerMail ID: drop the identity row. Mailboxes, local mail,
    /// and the GTE index are untouched (identity is independent of data sources).
    pub async fn clear(&self) -> AppResult<()> {
        sqlx::query("DELETE FROM seekermail_id")
            .execute(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(())
    }

    /// Set or withdraw the marketing-consent flag (opt-in; default OFF). Returns the
    /// updated identity, or `None` when signed out (no row to update).
    pub async fn set_marketing_consent(
        &self,
        consent: bool,
        source: Option<String>,
    ) -> AppResult<Option<SeekerMailId>> {
        let now = now_unix();
        sqlx::query(
            "UPDATE seekermail_id SET marketing_consent = ?, marketing_consent_source = ?, \
             marketing_consent_at = ?, updated_at = ? WHERE id = 'self'",
        )
        .bind(consent as i64)
        .bind(&source)
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        self.get().await
    }
}

/// DB projection of the single identity row (the columns the [`SeekerMailId`] DTO
/// needs). Booleans are stored as `INTEGER` and converted in [`From`].
#[derive(sqlx::FromRow)]
struct IdentityRow {
    provider: String,
    email: String,
    display_name: Option<String>,
    email_verified: i64,
    plan: Option<String>,
    marketing_consent: i64,
    marketing_consent_source: Option<String>,
    signed_in_at: i64,
}

impl From<IdentityRow> for SeekerMailId {
    fn from(r: IdentityRow) -> Self {
        SeekerMailId {
            provider: r.provider,
            email: r.email,
            display_name: r.display_name,
            email_verified: r.email_verified != 0,
            plan: r.plan,
            marketing_consent: r.marketing_consent != 0,
            marketing_consent_source: r.marketing_consent_source,
            signed_in_at: r.signed_in_at,
        }
    }
}

/// Verified claims for a successful sign-in, built from the OIDC `id_token` once the
/// cloud-identity backend lands (T121). See [`IdentityRepo::upsert_signin`].
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SignInInput {
    pub provider: String,
    pub provider_subject: String,
    pub email: String,
    pub email_verified: bool,
    pub display_name: Option<String>,
    pub plan: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> Db {
        let db = Db::connect_in_memory().await.expect("connect");
        db.run_migrations().await.expect("migrate");
        db
    }

    #[tokio::test]
    async fn signed_out_by_default_then_signin_signout_roundtrip() {
        let db = db().await;
        let repo = IdentityRepo::new(&db);
        assert!(
            repo.get().await.unwrap().is_none(),
            "no identity by default"
        );

        let id = repo
            .upsert_signin(
                &SignInInput {
                    provider: "google".into(),
                    provider_subject: "sub-123".into(),
                    email: "victor@example.com".into(),
                    email_verified: true,
                    display_name: Some("Victor".into()),
                    plan: None,
                },
                false,
                None,
            )
            .await
            .unwrap();
        assert_eq!(id.email, "victor@example.com");
        assert!(!id.marketing_consent, "consent defaults OFF");

        let updated = repo
            .set_marketing_consent(true, Some("settings".into()))
            .await
            .unwrap()
            .unwrap();
        assert!(updated.marketing_consent);

        repo.clear().await.unwrap();
        assert!(repo.get().await.unwrap().is_none(), "cleared on sign-out");
    }

    #[tokio::test]
    async fn single_row_is_enforced() {
        let db = db().await;
        let repo = IdentityRepo::new(&db);
        let input = SignInInput {
            provider: "google".into(),
            provider_subject: "sub-1".into(),
            email: "a@example.com".into(),
            email_verified: true,
            display_name: None,
            plan: None,
        };
        repo.upsert_signin(&input, false, None).await.unwrap();
        // A second sign-in replaces the row rather than adding one.
        let input2 = SignInInput {
            email: "b@example.com".into(),
            ..input
        };
        repo.upsert_signin(&input2, false, None).await.unwrap();
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM seekermail_id")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(n, 1, "exactly one identity row");
        assert_eq!(repo.get().await.unwrap().unwrap().email, "b@example.com");
    }
}
