//! `AccountService` — account lifecycle orchestration (T013/T014/T015/T018).
//!
//! The command layer stays thin (deser → one service call → map error); all the
//! business rules live here: validation, autodiscover defaults, credential
//! routing to the Keychain (never the DB), and the careful delete ordering that
//! purges secrets + blobs + vectors before the cascading row delete.

pub mod oauth;
pub mod pkce;
pub mod presets;
pub mod refresh;

use std::time::Duration;

pub use oauth::PendingOAuth;
pub use presets::autodiscover;

use crate::config::CONNECTION_TEST_TIMEOUT_SECS;
use crate::error::{AppError, AppResult};
use crate::keychain::{CredKind, Secret};
use crate::net::{ConnProbeConfig, ImapCreds, SmtpCreds};
use crate::state::AppState;
use crate::storage::account_repo::{AccountRepo, NewAccount};
use crate::types::{
    Account, CreateAccountParams, Provider, UpdateAccountParams, VerifyConnectionParams,
    VerifyConnectionResult,
};
use crate::util::{new_uuid, normalize_email, parse_uuid};

const VALID_COLOR_TOKENS: &[&str] = &["terra", "slate", "sage"];

/// Validate an account's color token (design-system invariant, F_A1 §4.7).
fn validate_color_token(token: &str) -> AppResult<()> {
    if VALID_COLOR_TOKENS.contains(&token) {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "color_token must be one of terra|slate|sage, got '{token}'"
        )))
    }
}

/// A badge label must be exactly one Unicode scalar value.
fn validate_badge_label(label: &str) -> AppResult<()> {
    if label.chars().count() == 1 {
        Ok(())
    } else {
        Err(AppError::Validation(
            "badge_label must be exactly one character".into(),
        ))
    }
}

fn validate_auth_level(level: u8) -> AppResult<()> {
    if (1..=3).contains(&level) {
        Ok(())
    } else {
        Err(AppError::Validation("auth_level must be 1, 2, or 3".into()))
    }
}

/// Post a TEAM-channel member-change system message (T094, F_I2 §3.3). Best
/// effort: any failure is swallowed so account create/delete never fails because
/// of a channel write. `verb` is `"joined"` / `"left"`.
async fn post_member_change(state: &AppState, verb: &str, display_name: &str, email: &str) {
    let text = format!("{display_name} ({email}) {verb} the collaboration channel.");
    let content = serde_json::json!({ "text": text }).to_string();
    let _ = crate::storage::im_repo::insert_message(
        state.storage.db(),
        "main",
        "system",
        "system",
        "text",
        &content,
        None,
        None,
    )
    .await;
}

/// Stateless service over [`AppState`].
pub struct AccountService;

impl AccountService {
    pub async fn list(state: &AppState) -> AppResult<Vec<Account>> {
        AccountRepo::new(state.storage.db()).list().await
    }

    pub async fn get(state: &AppState, id: &str) -> AppResult<Account> {
        AccountRepo::new(state.storage.db()).get(id).await
    }

    /// Provider hints for an email domain (T014). Thin pass-through so the wizard
    /// can pre-fill server fields.
    pub fn autodiscover(email: &str) -> Option<crate::types::ProviderHints> {
        presets::autodiscover(email)
    }

    /// Create an account + store its password in the Keychain (T013). The first
    /// account becomes primary automatically (enforced in the repo transaction).
    pub async fn create(state: &AppState, params: CreateAccountParams) -> AppResult<Account> {
        validate_color_token(&params.color_token)?;
        validate_badge_label(&params.badge_label)?;
        let auth_level = params.auth_level.unwrap_or(1);
        validate_auth_level(auth_level)?;

        let email = normalize_email(&params.email);
        let hints = presets::autodiscover(&email);

        // Resolve server coordinates: explicit → preset → (for IMAP) error.
        let imap_host = params
            .imap_host
            .clone()
            .or_else(|| hints.as_ref().map(|h| h.imap_host.clone()));
        let smtp_host = params
            .smtp_host
            .clone()
            .or_else(|| hints.as_ref().map(|h| h.smtp_host.clone()));
        if matches!(params.provider, Provider::Imap) && imap_host.is_none() {
            return Err(AppError::Validation(
                "imap_host is required for IMAP accounts".into(),
            ));
        }
        let imap_port = params
            .imap_port
            .or_else(|| hints.as_ref().map(|h| h.imap_port))
            .unwrap_or(993);
        let smtp_port = params
            .smtp_port
            .or_else(|| hints.as_ref().map(|h| h.smtp_port))
            .unwrap_or(587);

        let id = new_uuid();
        let new = NewAccount {
            id: id.clone(),
            email,
            display_name: params.display_name,
            provider: params.provider.as_str().to_string(),
            imap_host,
            imap_port,
            smtp_host,
            smtp_port,
            color_token: params.color_token,
            badge_label: params.badge_label,
            role_type: params.role_type.unwrap_or_else(|| "custom".into()),
            role_description: params.role_description,
            auth_level,
        };
        let account = AccountRepo::new(state.storage.db()).create(&new).await?;

        // Password (IMAP/SMTP only) → Keychain, never the DB.
        if !params.provider.is_oauth() {
            if let Some(pw) = params.password {
                let uuid = parse_uuid(&id)?;
                let secret = Secret::new(pw);
                state.keychain.set(&uuid, CredKind::ImapPassword, &secret)?;
                state.keychain.set(&uuid, CredKind::SmtpPassword, &secret)?;
            }
        }

        // T094: announce the new agent in the TEAM channel. Best-effort — a failed
        // system message must never fail account creation (F_I2 §3.3).
        post_member_change(state, "joined", &account.display_name, &account.email).await;
        Ok(account)
    }

    /// Apply a partial update (T013).
    pub async fn update(
        state: &AppState,
        id: &str,
        patch: UpdateAccountParams,
    ) -> AppResult<Account> {
        if let Some(c) = &patch.color_token {
            validate_color_token(c)?;
        }
        if let Some(b) = &patch.badge_label {
            validate_badge_label(b)?;
        }
        if let Some(l) = patch.auth_level {
            validate_auth_level(l)?;
        }
        AccountRepo::new(state.storage.db())
            .update(id, &patch)
            .await
    }

    /// Promote one account to primary (T091). The single-primary invariant is
    /// enforced atomically in the repo transaction (see [`AccountRepo::set_primary`]).
    pub async fn set_primary(state: &AppState, id: &str) -> AppResult<Account> {
        AccountRepo::new(state.storage.db()).set_primary(id).await
    }

    /// Startup self-heal for the single-primary invariant (T091, F_I1 §6). Logs a
    /// warning when it had to repair a 0- or ≥2-primary database.
    pub async fn heal_primary(state: &AppState) -> AppResult<()> {
        let before = AccountRepo::new(state.storage.db()).heal_primary().await?;
        if before != 1 {
            tracing::warn!(
                primaries_before = before,
                "healed primary account invariant (expected exactly 1)"
            );
        }
        Ok(())
    }

    /// Toggle active state — the backing of `enable_account` / `disable_account`.
    pub async fn set_active(state: &AppState, id: &str, active: bool) -> AppResult<Account> {
        let patch = UpdateAccountParams {
            is_active: Some(active),
            ..Default::default()
        };
        AccountRepo::new(state.storage.db())
            .update(id, &patch)
            .await
    }

    /// Replace the stored IMAP/SMTP password (T013 `update_account_password`).
    pub async fn update_password(state: &AppState, id: &str, password: String) -> AppResult<()> {
        let uuid = parse_uuid(id)?;
        let secret = Secret::new(password);
        state.keychain.set(&uuid, CredKind::ImapPassword, &secret)?;
        state.keychain.set(&uuid, CredKind::SmtpPassword, &secret)?;
        Ok(())
    }

    /// Delete one account / disconnect one mailbox (T026 §6). In the decoupled
    /// SeekerMail ID model (A6, analysis/26) a mailbox is just a data source, so
    /// removing the **last** one is allowed — it leaves zero mailboxes, a valid
    /// state. The independent SeekerMail ID is unaffected; signing out of the ID is
    /// a separate action (see `commands::identity`). Purges the account's
    /// secrets/blobs/vectors/rows, heals the single-primary invariant, then
    /// announces the departure in the TEAM channel.
    pub async fn delete(state: &AppState, id: &str) -> AppResult<()> {
        let repo = AccountRepo::new(state.storage.db());
        let account = Self::purge_account(state, id).await?;
        // T091 invariant backstop: if the deleted account was primary, the db now
        // has zero primaries — promote the earliest remaining account so the
        // single-primary invariant always holds, even if a caller bypasses the
        // frontend "switch primary first" guard (F_I1 §6).
        repo.heal_primary().await?;
        // T094: announce the departure in the TEAM channel (best-effort).
        post_member_change(state, "left", &account.display_name, &account.email).await;
        Ok(())
    }

    /// Purge one account's secrets, blob files, derived vectors, and DB rows in the
    /// delete ordering (T026 §6): Keychain first (no orphan secrets survive) → blob
    /// files on disk → derived vectors → DB rows (ON DELETE CASCADE clears
    /// threads/mails/sync_state/…). Teardown for [`Self::delete`]; it does **not**
    /// heal the primary invariant or post a channel message — the caller decides
    /// those. Returns the account identity (for any post-delete messaging).
    async fn purge_account(state: &AppState, id: &str) -> AppResult<Account> {
        let repo = AccountRepo::new(state.storage.db());
        // Ensure it exists (clear NOT_FOUND semantics) + capture identity.
        let account = repo.get(id).await?;
        let uuid = parse_uuid(id)?;
        state.keychain.delete_all(&uuid)?;
        state.storage.blobs().cleanup_account_dir(id).await?;
        state.storage.vectors().delete_account(id)?;
        repo.delete(id).await?;
        Ok(account)
    }

    /// Set the account's knowledge-depth (T016). `None` = all mail.
    pub async fn set_knowledge_depth(
        state: &AppState,
        id: &str,
        months: Option<u32>,
    ) -> AppResult<Account> {
        if let Some(m) = months {
            if ![3, 6, 12, 36, 60].contains(&m) {
                return Err(AppError::Validation(
                    "knowledge depth must be 3/6/12/36/60 months or all".into(),
                ));
            }
        }
        AccountRepo::new(state.storage.db())
            .set_knowledge_depth(id, months)
            .await
    }

    /// Probe IMAP + SMTP (T014). In-band result: `Ok(_)` even when both probes
    /// fail; only an internal fault produces an `Err`.
    pub async fn verify_connection(
        state: &AppState,
        params: VerifyConnectionParams,
    ) -> AppResult<VerifyConnectionResult> {
        let email = normalize_email(&params.email);
        let hints = presets::autodiscover(&email);

        let imap_host = params
            .imap_host
            .clone()
            .or_else(|| hints.as_ref().map(|h| h.imap_host.clone()))
            .unwrap_or_default();
        let smtp_host = params
            .smtp_host
            .clone()
            .or_else(|| hints.as_ref().map(|h| h.smtp_host.clone()))
            .unwrap_or_default();
        let imap_port = params
            .imap_port
            .or_else(|| hints.as_ref().map(|h| h.imap_port))
            .unwrap_or(993);
        let smtp_port = params
            .smtp_port
            .or_else(|| hints.as_ref().map(|h| h.smtp_port))
            .unwrap_or(587);
        let secret = params.password.clone().unwrap_or_default();

        let cfg = ConnProbeConfig {
            imap: ImapCreds {
                host: imap_host,
                port: imap_port,
                tls: params.imap_tls.unwrap_or(imap_port == 993),
                email: email.clone(),
                secret: secret.clone(),
            },
            smtp: SmtpCreds {
                host: smtp_host,
                port: smtp_port,
                tls: params.smtp_tls.unwrap_or(true),
                email,
                secret,
            },
        };

        let probe = state.net.probe.clone();
        let report = tokio::time::timeout(
            Duration::from_secs(CONNECTION_TEST_TIMEOUT_SECS),
            probe.verify(cfg),
        )
        .await;

        Ok(match report {
            Ok(r) => VerifyConnectionResult {
                imap_ok: r.imap_ok,
                smtp_ok: r.smtp_ok,
                error_message: r.error_message,
            },
            Err(_) => VerifyConnectionResult {
                imap_ok: false,
                smtp_ok: false,
                error_message: Some("Connection timed out".into()),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_validates_and_persists() {
        let (state, _rx) = AppState::test_state().await;
        let params = CreateAccountParams {
            email: "Person@Gmail.com".into(),
            display_name: "Personal".into(),
            provider: Provider::Gmail,
            imap_host: None,
            imap_port: None,
            smtp_host: None,
            smtp_port: None,
            color_token: "sage".into(),
            badge_label: "P".into(),
            role_type: Some("personal".into()),
            role_description: None,
            auth_level: Some(2),
            password: None,
        };
        let a = AccountService::create(&state, params).await.unwrap();
        assert_eq!(a.email, "person@gmail.com"); // normalised
        assert_eq!(a.imap_host.as_deref(), Some("imap.gmail.com")); // autodiscovered
        assert!(a.is_primary);
    }

    #[tokio::test]
    async fn invalid_color_and_badge_rejected() {
        let (state, _rx) = AppState::test_state().await;
        let base = CreateAccountParams {
            email: "a@x.com".into(),
            display_name: "A".into(),
            provider: Provider::Imap,
            imap_host: Some("imap.x.com".into()),
            imap_port: Some(993),
            smtp_host: Some("smtp.x.com".into()),
            smtp_port: Some(587),
            color_token: "purple".into(),
            badge_label: "AB".into(),
            role_type: None,
            role_description: None,
            auth_level: None,
            password: Some("pw".into()),
        };
        assert!(matches!(
            AccountService::create(&state, base.clone())
                .await
                .unwrap_err(),
            AppError::Validation(_)
        ));
        let ok_color = CreateAccountParams {
            color_token: "slate".into(),
            ..base
        };
        // badge still invalid (2 chars).
        assert!(matches!(
            AccountService::create(&state, ok_color).await.unwrap_err(),
            AppError::Validation(_)
        ));
    }

    #[tokio::test]
    async fn delete_last_account_is_allowed() {
        // A6 decoupling (analysis/26): a mailbox is just a data source, so removing
        // the last one is a valid state (zero mailboxes). The old "can't remove your
        // only account" dead-end is gone — identity is independent of mailboxes.
        let (state, _rx) = AppState::test_state().await;
        AccountService::create(&state, imap_params("solo@x.com", "S"))
            .await
            .unwrap();
        let only = AccountService::list(&state).await.unwrap()[0].id.clone();
        AccountService::delete(&state, &only).await.unwrap();
        assert!(AccountService::list(&state).await.unwrap().is_empty());
    }

    /// Minimal IMAP account params (no password → no Keychain writes in tests).
    fn imap_params(email: &str, badge: &str) -> CreateAccountParams {
        CreateAccountParams {
            email: email.into(),
            display_name: email.split('@').next().unwrap_or("Acct").into(),
            provider: Provider::Imap,
            imap_host: Some("imap.x.com".into()),
            imap_port: Some(993),
            smtp_host: Some("smtp.x.com".into()),
            smtp_port: Some(587),
            color_token: "slate".into(),
            badge_label: badge.into(),
            role_type: None,
            role_description: None,
            auth_level: None,
            password: None,
        }
    }

    #[tokio::test]
    async fn verify_connection_is_in_band_offline() {
        let (state, _rx) = AppState::test_state().await;
        let res = AccountService::verify_connection(
            &state,
            VerifyConnectionParams {
                email: "a@gmail.com".into(),
                provider: Provider::Gmail,
                password: Some("x".into()),
                imap_host: None,
                imap_port: None,
                imap_tls: None,
                smtp_host: None,
                smtp_port: None,
                smtp_tls: None,
            },
        )
        .await
        .unwrap(); // Ok, not Err — in-band result.
        assert!(!res.imap_ok);
        assert!(res.error_message.is_some());
    }

    #[tokio::test]
    async fn verify_connection_reports_success_with_a_reachable_server() {
        // Inject a fake probe (both endpoints reachable) so the T014 success path
        // — never reachable through the offline adapter — is actually exercised.
        let net =
            crate::net::fakes::fake_net(None, Some(crate::net::fakes::FakeConnProbe::ok()), None);
        let (state, _rx) = AppState::test_state_with_net(net).await;
        let res = AccountService::verify_connection(
            &state,
            VerifyConnectionParams {
                email: "a@gmail.com".into(),
                provider: Provider::Gmail,
                password: Some("x".into()),
                imap_host: None,
                imap_port: None,
                imap_tls: None,
                smtp_host: None,
                smtp_port: None,
                smtp_tls: None,
            },
        )
        .await
        .unwrap();
        assert!(res.imap_ok);
        assert!(res.smtp_ok);
        assert!(res.error_message.is_none());
    }

    #[tokio::test]
    async fn create_posts_join_system_message() {
        let (state, _rx) = AppState::test_state().await;
        AccountService::create(
            &state,
            CreateAccountParams {
                email: "alex@northwind.co".into(),
                display_name: "Alex".into(),
                provider: Provider::Imap,
                imap_host: Some("imap.northwind.co".into()),
                imap_port: Some(993),
                smtp_host: Some("smtp.northwind.co".into()),
                smtp_port: Some(587),
                color_token: "slate".into(),
                badge_label: "A".into(),
                role_type: None,
                role_description: None,
                auth_level: None,
                password: None,
            },
        )
        .await
        .unwrap();

        let page =
            crate::storage::im_repo::list_messages(state.storage.db(), None, None, None, None)
                .await
                .unwrap();
        assert!(
            page.items.iter().any(|m| m.sender_type == "system"
                && m.content.contains("joined the collaboration channel")),
            "a join system message should be posted to the TEAM channel"
        );
    }
}
