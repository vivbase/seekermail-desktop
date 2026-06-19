//! F3 recommended-provider tiers (T064, F_F3 §3–§4.6, dev/06 §8).
//!
//! Two curated tiers — *balanced* (default) and *high quality* — let a user
//! enable AI without ever seeing an API key. The flow mirrors the account
//! OAuth PKCE grant (T015, `account::oauth`): system browser → deep-link
//! callback → token straight into the OS Keychain. ADR-0004 red line: the
//! token exchange and every subsequent AI request go device → provider; no
//! SeekerMail server is ever on the path.
//!
//! Vendor reality (F_F3 §4.1 note): the final recommended partners are a
//! business decision. The tier list is therefore **config, not behavior** —
//! [`RECOMMENDED_PROVIDERS`] carries the endpoints, scopes, and client-id env
//! names, so swapping a vendor or model slug is a one-constant change. Where
//! the spec leaves endpoints abstract they live here, never inline in flow
//! code.
//!
//! Conservative first-week quota (F_F3 §4.6): the first successful
//! authorization stamps `app_settings["ai.first_auth_at"]` and arms
//! `app_settings["ai.conservative_quota_until"]` (now + 7 days). While armed,
//! [`super::registry::AiRegistry::resolve`] caps the per-account daily limit
//! at [`CONSERVATIVE_DAILY_LIMIT`] and
//! [`super::registry::AiRegistry::clamp_chat_request`] caps `max_tokens` at
//! [`CONSERVATIVE_MAX_TOKENS`]. The settings page can lift the cap early by
//! deleting the key (`clear_conservative_quota` command).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use specta::Type;
use zeroize::Zeroize;

use crate::account::pkce::new_pkce;
use crate::ai::matrix::build_default_matrix;
use crate::ai::providers;
use crate::ai::registry::AccountAiConfig;
use crate::error::{AppError, AppResult};
use crate::keychain::{CredKind, Secret};
use crate::state::AppState;
use crate::storage::settings_repo::SettingRepo;
use crate::storage::{map_sqlx_err, Db};
use crate::types::AiProvider;
use crate::util::{now_unix, parse_uuid};

// ── app_settings keys (backend-owned `ai.` namespace) ───────────────────────

/// Unix timestamp of the very first successful recommended-provider
/// authorization. Written once; never moved by later re-authorizations.
pub const FIRST_AUTH_AT_KEY: &str = "ai.first_auth_at";

/// Unix timestamp until which the conservative first-week quota applies
/// (F_F3 §4.6). Absent or in the past = no cap. The settings page lifts the
/// cap by deleting the key.
pub const CONSERVATIVE_QUOTA_UNTIL_KEY: &str = "ai.conservative_quota_until";

/// Unix timestamp of the user's data-flow disclosure confirmation
/// (dev/06 §8). Absent = the non-bypassable modal has not been confirmed and
/// no cloud recommended tier may begin authorization.
pub const DISCLOSURE_CONFIRMED_AT_KEY: &str = "ai.disclosure_confirmed_at";

/// Daily cloud-call cap while the conservative quota is armed (F_F3 §4.6).
pub const CONSERVATIVE_DAILY_LIMIT: i64 = 100;

/// `max_tokens` cap while the conservative quota is armed (F_F3 §4.6).
pub const CONSERVATIVE_MAX_TOKENS: u32 = 2_000;

/// Conservative-quota window length: 7 days (F_F3 §4.6).
pub const CONSERVATIVE_QUOTA_SECS: i64 = 7 * 86_400;

/// A pending recommended-OAuth grant expires after 5 minutes (card §6).
const PENDING_TTL_SECS: i64 = 300;

/// Deep-link redirect for the recommended-provider grant. Deliberately a
/// *different path* from the account-mail callback
/// (`seekermail://oauth/callback`, T015) so the deep-link handler can route a
/// callback to the right flow without guessing which pending store owns the
/// state nonce.
pub const RECOMMENDED_REDIRECT_URI: &str = "seekermail://oauth/recommended";

/// Tauri event the deep-link handler emits toward the wizard once it has
/// parsed a recommended callback (card §6). Payload: [`RecommendedOAuthCallback`].
pub const OAUTH_CALLBACK_EVENT: &str = "oauth:callback";

// ── Tier configuration ───────────────────────────────────────────────────────

/// The two recommendation tiers (F_F3 §3 step 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RecommendedTier {
    Balanced,
    HighQuality,
}

impl RecommendedTier {
    /// Stable wire/log tag (mirrors the serde representation).
    pub fn as_str(self) -> &'static str {
        match self {
            RecommendedTier::Balanced => "balanced",
            RecommendedTier::HighQuality => "high_quality",
        }
    }

    /// Parse the wire tag; unknown values are a validation error (the tier
    /// comes from the UI, so an unknown tag means a stale frontend).
    pub fn parse(s: &str) -> AppResult<Self> {
        match s {
            "balanced" => Ok(RecommendedTier::Balanced),
            "high_quality" => Ok(RecommendedTier::HighQuality),
            other => Err(AppError::Validation(format!(
                "unknown recommended tier: {other}"
            ))),
        }
    }
}

/// One entry of the built-in recommendation list (F_F3 §4.1). Static for
/// v0.5; the field shapes are deliberately data-only so a later version can
/// load an override list from `app_settings["ai.recommended_providers_override"]`
/// without touching flow code (card §6).
#[derive(Debug, Clone, Copy)]
pub struct RecommendedProvider {
    pub tier: RecommendedTier,
    pub provider: AiProvider,
    /// Model slug at the provider. Placeholder pending partner confirmation —
    /// changing the vendor/model is a one-constant edit (F_F3 §4.1 note).
    pub model: &'static str,
    pub display_name: &'static str,
    /// Typical monthly cost band in USD for ~200 auto-replies (F_F3 §4.4).
    pub monthly_cost_range_usd: (f32, f32),
    /// Average token spend per AI reply, for the cost-estimate copy.
    pub tokens_per_reply_estimate: u32,
    /// OAuth authorization endpoint. `None` = the vendor offers no OAuth and
    /// the wizard must fall back to the F_F3 §4.2 authorization-code paste.
    pub authorize_url: Option<&'static str>,
    /// OAuth token-exchange endpoint (direct HTTPS, device → vendor).
    pub token_url: Option<&'static str>,
    /// Token revocation endpoint, when the vendor offers one (F_F3 §4.5).
    /// `None` = local-only revoke + point the user at the vendor console.
    pub revoke_url: Option<&'static str>,
    /// Env var carrying the partner-issued OAuth client id. Runtime-read (the
    /// T015 pattern) so builds without the secret still compile and fail with
    /// a clean `AUTH_OAUTH_FAILED`.
    pub client_id_env: &'static str,
    /// OAuth scope string requested at authorization.
    pub scope: &'static str,
}

/// The built-in recommendation list (F_F3 §4.1). Vendors are the v0.5
/// provisional picks (balanced = Anthropic mid-tier, high quality = OpenAI
/// flagship); both are stand-ins until the partnership is confirmed.
pub const RECOMMENDED_PROVIDERS: &[RecommendedProvider] = &[
    RecommendedProvider {
        tier: RecommendedTier::Balanced,
        provider: AiProvider::Anthropic,
        model: "claude-sonnet-4-5",
        display_name: "Anthropic Claude (balanced)",
        monthly_cost_range_usd: (3.0, 9.0),
        tokens_per_reply_estimate: 1_500,
        authorize_url: Some("https://claude.ai/oauth/authorize"),
        token_url: Some("https://console.anthropic.com/v1/oauth/token"),
        revoke_url: None,
        client_id_env: "SEEKERMAIL_ANTHROPIC_OAUTH_CLIENT_ID",
        scope: "user:inference",
    },
    RecommendedProvider {
        tier: RecommendedTier::HighQuality,
        provider: AiProvider::Openai,
        model: "gpt-5",
        display_name: "OpenAI flagship (high quality)",
        monthly_cost_range_usd: (12.0, 30.0),
        tokens_per_reply_estimate: 1_800,
        authorize_url: Some("https://auth.openai.com/authorize"),
        token_url: Some("https://auth.openai.com/oauth/token"),
        revoke_url: None,
        client_id_env: "SEEKERMAIL_OPENAI_OAUTH_CLIENT_ID",
        scope: "model.request",
    },
];

/// The config entry for one tier.
pub fn tier_config(tier: RecommendedTier) -> &'static RecommendedProvider {
    RECOMMENDED_PROVIDERS
        .iter()
        .find(|p| p.tier == tier)
        .expect("every RecommendedTier has a RECOMMENDED_PROVIDERS entry")
}

// ── Wire DTOs (exported to bindings; serde camelCase + specta) ───────────────

/// What the wizard renders per tier (F_F3 §3 step 2 + §4.4). Endpoint URLs
/// and env names are deliberately absent — the frontend never needs them.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RecommendedProviderInfo {
    pub tier: RecommendedTier,
    pub provider: AiProvider,
    pub model: String,
    pub display_name: String,
    pub monthly_cost_min_usd: f32,
    pub monthly_cost_max_usd: f32,
    pub tokens_per_reply_estimate: u32,
    /// `false` = the wizard must use the authorization-code paste fallback.
    pub oauth_supported: bool,
}

impl From<&RecommendedProvider> for RecommendedProviderInfo {
    fn from(p: &RecommendedProvider) -> Self {
        RecommendedProviderInfo {
            tier: p.tier,
            provider: p.provider,
            model: p.model.to_string(),
            display_name: p.display_name.to_string(),
            monthly_cost_min_usd: p.monthly_cost_range_usd.0,
            monthly_cost_max_usd: p.monthly_cost_range_usd.1,
            tokens_per_reply_estimate: p.tokens_per_reply_estimate,
            oauth_supported: p.authorize_url.is_some(),
        }
    }
}

/// The public recommendation list (no endpoints, no client ids).
pub fn recommended_provider_infos() -> Vec<RecommendedProviderInfo> {
    RECOMMENDED_PROVIDERS.iter().map(Into::into).collect()
}

/// `begin_recommended_oauth` result: the state nonce the wizard must hand
/// back at completion, plus the URL that was opened in the system browser.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BeginRecommendedOAuthResult {
    pub state: String,
    pub authorize_url: String,
}

/// `complete_recommended_oauth` result (card §3). In-band like
/// `verify_ai_provider`: a failed connection test is `ok = false`, not an IPC
/// error — hard grant failures (state mismatch, exchange refusal) stay errors.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CompleteRecommendedOAuthResult {
    pub ok: bool,
    pub provider_name: String,
    pub model_name: Option<String>,
    pub error_message: Option<String>,
}

/// Setup-state snapshot for the wizard + settings page: has the disclosure
/// been confirmed, is the conservative quota armed, when was first auth.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiSetupStatus {
    pub disclosure_confirmed_at: Option<i64>,
    pub conservative_quota_until: Option<i64>,
    pub first_auth_at: Option<i64>,
}

/// Payload of the [`OAUTH_CALLBACK_EVENT`] the deep-link handler emits.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RecommendedOAuthCallback {
    pub code: String,
    pub state: String,
}

/// Parse a `seekermail://oauth/recommended?code=…&state=…` deep link. The
/// lib.rs URL-scheme handler calls this to decide whether a callback belongs
/// to the recommended flow (vs. the account-mail flow on `/oauth/callback`).
pub fn parse_recommended_callback(url: &str) -> Option<RecommendedOAuthCallback> {
    let rest = url.strip_prefix(RECOMMENDED_REDIRECT_URI)?;
    let query = rest.strip_prefix('?')?;
    let mut code = None;
    let mut state = None;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=')?;
        match k {
            "code" => code = Some(v.to_string()),
            "state" => state = Some(v.to_string()),
            _ => {}
        }
    }
    Some(RecommendedOAuthCallback {
        code: code?,
        state: state?,
    })
}

// ── Pending-grant store (CSRF state, card §6) ────────────────────────────────

/// One in-flight recommended grant, keyed by its state nonce.
#[derive(Debug, Clone)]
struct PendingRecommendedAuth {
    tier: RecommendedTier,
    verifier: String,
    created_at: i64,
}

fn pending_store() -> &'static Mutex<HashMap<String, PendingRecommendedAuth>> {
    static PENDING: OnceLock<Mutex<HashMap<String, PendingRecommendedAuth>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

fn park_pending(state_nonce: &str, pending: PendingRecommendedAuth) {
    let mut map = pending_store().lock().expect("pending oauth map poisoned");
    // Opportunistic GC so abandoned grants never accumulate.
    let now = now_unix();
    map.retain(|_, p| now - p.created_at <= PENDING_TTL_SECS);
    map.insert(state_nonce.to_string(), pending);
}

/// Take (and consume) the pending grant for a returned state nonce. A miss or
/// an expired entry is `AUTH_OAUTH_FAILED` — the CSRF check (card §6).
fn take_pending(state_nonce: &str) -> AppResult<PendingRecommendedAuth> {
    let pending = pending_store()
        .lock()
        .expect("pending oauth map poisoned")
        .remove(state_nonce)
        .ok_or_else(|| AppError::AuthOAuthFailed("unknown or already-used oauth state".into()))?;
    if now_unix() - pending.created_at > PENDING_TTL_SECS {
        return Err(AppError::AuthOAuthFailed("oauth grant expired".into()));
    }
    Ok(pending)
}

// ── Quota / disclosure readers (shared with the registry) ────────────────────

/// JSON-number read of one `app_settings` timestamp key.
async fn read_ts_key(db: &Db, key: &str) -> AppResult<Option<i64>> {
    let raw = SettingRepo::new(db).get(key).await?;
    Ok(raw.and_then(|v| serde_json::from_str::<i64>(&v).ok()))
}

/// The conservative-quota deadline, if one is stored.
pub async fn conservative_quota_until(db: &Db) -> AppResult<Option<i64>> {
    read_ts_key(db, CONSERVATIVE_QUOTA_UNTIL_KEY).await
}

/// Is the first-week conservative quota currently armed? (F_F3 §4.6 — a past
/// timestamp counts as lifted, which is also how the settings page disarms it.)
pub async fn conservative_quota_active(db: &Db) -> AppResult<bool> {
    Ok(matches!(
        conservative_quota_until(db).await?,
        Some(until) if now_unix() < until
    ))
}

/// Current setup snapshot for the wizard and the settings page.
pub async fn setup_status(db: &Db) -> AppResult<AiSetupStatus> {
    Ok(AiSetupStatus {
        disclosure_confirmed_at: read_ts_key(db, DISCLOSURE_CONFIRMED_AT_KEY).await?,
        conservative_quota_until: read_ts_key(db, CONSERVATIVE_QUOTA_UNTIL_KEY).await?,
        first_auth_at: read_ts_key(db, FIRST_AUTH_AT_KEY).await?,
    })
}

/// Record the user's data-flow disclosure confirmation (dev/06 §8). The
/// timestamp is the auditable record; `update_account_ai_settings` resets it
/// when the endpoint changes (that reset belongs to the F1 surface).
pub async fn confirm_disclosure(db: &Db) -> AppResult<AiSetupStatus> {
    SettingRepo::new(db)
        .set(DISCLOSURE_CONFIRMED_AT_KEY, &now_unix().to_string())
        .await?;
    tracing::info!(
        event = "ai_disclosure_confirmed",
        "data-flow disclosure confirmed by the user"
    );
    setup_status(db).await
}

/// Lift the first-week conservative quota early (F_F3 §4.6 — the settings
/// page control). Deleting the key is equivalent to a past timestamp.
pub async fn clear_conservative_quota(db: &Db) -> AppResult<()> {
    SettingRepo::new(db)
        .delete(CONSERVATIVE_QUOTA_UNTIL_KEY)
        .await?;
    tracing::info!(
        event = "ai_conservative_quota_cleared",
        "first-week conservative quota lifted by the user"
    );
    Ok(())
}

// ── OAuth flow ───────────────────────────────────────────────────────────────

/// Minimal RFC-3986 percent-encoding for query values (unreserved pass).
fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Build the vendor authorization URL with all PKCE parameters (mirrors
/// `account::oauth::build_authorize_url`).
fn build_authorize_url(
    cfg: &RecommendedProvider,
    client_id: &str,
    challenge: &str,
    state_nonce: &str,
) -> AppResult<String> {
    let authorize = cfg.authorize_url.ok_or_else(|| {
        AppError::AuthOAuthFailed(format!(
            "tier '{}' has no oauth endpoint configured; use the authorization-code fallback",
            cfg.tier.as_str()
        ))
    })?;
    Ok(format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
        authorize,
        pct(client_id),
        pct(RECOMMENDED_REDIRECT_URI),
        pct(cfg.scope),
        pct(challenge),
        pct(state_nonce),
    ))
}

/// Runtime read of the partner-issued OAuth client id (T015 pattern: a build
/// without the secret compiles and degrades to a clean `AUTH_OAUTH_FAILED`).
fn client_id(cfg: &RecommendedProvider) -> AppResult<String> {
    std::env::var(cfg.client_id_env)
        .map_err(|_| AppError::AuthOAuthFailed(format!("{} is not configured", cfg.client_id_env)))
}

/// Begin a recommended grant: enforce the disclosure gate, generate PKCE
/// material, park the pending state, and return the authorize URL for the
/// caller to open in the **system browser** (never an embedded webview).
pub async fn begin(
    state: &AppState,
    tier: RecommendedTier,
) -> AppResult<BeginRecommendedOAuthResult> {
    let cfg = tier_config(tier);

    // dev/06 §8 — the disclosure modal is non-bypassable for cloud tiers: the
    // backend refuses to even start a grant before the confirmation exists.
    if cfg.provider.is_cloud()
        && read_ts_key(state.storage.db(), DISCLOSURE_CONFIRMED_AT_KEY)
            .await?
            .is_none()
    {
        return Err(AppError::Forbidden(
            "the data-flow disclosure must be confirmed before a cloud provider is authorized"
                .into(),
        ));
    }

    let cid = client_id(cfg)?;
    let pkce = new_pkce();
    let url = build_authorize_url(cfg, &cid, &pkce.challenge, &pkce.state)?;
    park_pending(
        &pkce.state,
        PendingRecommendedAuth {
            tier,
            verifier: pkce.verifier,
            created_at: now_unix(),
        },
    );
    tracing::info!(
        event = "recommended_oauth_begin",
        tier = tier.as_str(),
        provider = cfg.provider.as_str(),
        "recommended-provider oauth grant started"
    );
    Ok(BeginRecommendedOAuthResult {
        state: pkce.state,
        authorize_url: url,
    })
}

/// All account ids, for the F4 default fill (F_F3 §4.3 — every account starts
/// using the selected provider).
async fn all_account_ids(db: &Db) -> AppResult<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT id FROM accounts")
        .fetch_all(db.pool())
        .await
        .map_err(map_sqlx_err)?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// DB-side effects of a successful authorization (keychain writes live in
/// [`complete`] so this stays hermetically testable):
///
/// * every account's `account_ai_settings` row is upserted to the selected
///   provider/model with `ai_api_key_ref = account_id` (F_F3 §4.3);
/// * each account's F4 matrix is filled with the computed defaults — E4 keeps
///   `local_onnx` when that adapter is registered (F_F3 §4.3 exception);
/// * `ai.first_auth_at` is stamped once, and only that first stamp arms the
///   7-day conservative quota (F_F3 §4.6).
pub async fn apply_recommended_defaults(
    state: &AppState,
    cfg: &RecommendedProvider,
) -> AppResult<()> {
    let db = state.storage.db();
    let now = now_unix();
    let registered = state.ai.registered();

    for account_id in all_account_ids(db).await? {
        // Materialize a settings row if account creation predates the AI schema.
        sqlx::query(
            "INSERT OR IGNORE INTO account_ai_settings (account_id, auth_level, updated_at) \
             VALUES (?, 1, ?)",
        )
        .bind(&account_id)
        .bind(now)
        .execute(db.pool())
        .await
        .map_err(map_sqlx_err)?;

        let base = AccountAiConfig {
            account_id: account_id.clone(),
            provider: cfg.provider,
            model: Some(cfg.model.to_string()),
            base_url: None,
            api_key_ref: Some(account_id.clone()),
            daily_query_limit: 0, // not consulted by build_default_matrix
            updated_at: now,
        };
        let matrix = build_default_matrix(&base, &registered);

        sqlx::query(
            "UPDATE account_ai_settings SET ai_provider = ?, ai_model = ?, ai_base_url = NULL, \
                 ai_api_key_ref = ?, provider_matrix = ?, updated_at = ? \
             WHERE account_id = ?",
        )
        .bind(cfg.provider.as_str())
        .bind(cfg.model)
        .bind(&account_id)
        .bind(matrix.to_json())
        .bind(now)
        .bind(&account_id)
        .execute(db.pool())
        .await
        .map_err(map_sqlx_err)?;
    }

    let repo = SettingRepo::new(db);
    if read_ts_key(db, FIRST_AUTH_AT_KEY).await?.is_none() {
        repo.set(FIRST_AUTH_AT_KEY, &now.to_string()).await?;
        repo.set(
            CONSERVATIVE_QUOTA_UNTIL_KEY,
            &(now + CONSERVATIVE_QUOTA_SECS).to_string(),
        )
        .await?;
    }

    // Identifiers only — never tokens or mail content (09 §5).
    tracing::info!(
        event = "recommended_defaults_applied",
        tier = cfg.tier.as_str(),
        provider = cfg.provider.as_str(),
        "recommended provider applied as the default across accounts"
    );
    Ok(())
}

/// In-band connection probe with the freshly granted token (F_F3 §3 step 4).
/// Mirrors `verify_ai_provider`: failures come back as `ok = false` with the
/// sanitized, content-free `ProviderError` rendering.
async fn probe_with_token(
    cfg: &RecommendedProvider,
    token: &str,
) -> (bool, Option<String>, Option<String>) {
    let outcome = match cfg.provider {
        AiProvider::Openai => providers::openai::probe(cfg.model, Some(token), None).await,
        AiProvider::Anthropic => {
            providers::anthropic::AnthropicClient::probe(cfg.model, Some(token), None).await
        }
        _ => {
            return (
                false,
                None,
                Some("this provider cannot be verified over the network".into()),
            )
        }
    };
    match outcome {
        Ok(health) => (health.ok, health.model_name, None),
        Err(err) => (false, None, Some(err.to_string())),
    }
}

/// Complete a recommended grant from the deep-link callback (or the manual
/// code paste, F_F3 §6): CSRF-validate the state nonce, exchange the code at
/// the vendor token endpoint (direct HTTPS), store the token in the Keychain
/// for every account, fill the F4 defaults, arm the first-week quota, and run
/// the connection test. Token plaintext is zeroized the moment it has served
/// its two purposes (Keychain write + probe) — it never reaches the DB, a log
/// line, or the IPC return value.
pub async fn complete(
    state: &AppState,
    state_nonce: &str,
    code: &str,
) -> AppResult<CompleteRecommendedOAuthResult> {
    let pending = take_pending(state_nonce)?;
    let cfg = tier_config(pending.tier);
    let cid = client_id(cfg)?;
    let token_url = cfg.token_url.ok_or_else(|| {
        AppError::AuthOAuthFailed(format!(
            "tier '{}' has no token endpoint configured",
            cfg.tier.as_str()
        ))
    })?;

    // Direct device → vendor exchange via the transport seam (ADR-0004).
    let req = crate::net::TokenRequest {
        token_url: token_url.to_string(),
        client_id: cid,
        redirect_uri: RECOMMENDED_REDIRECT_URI.to_string(),
        code: Some(code.to_string()),
        code_verifier: Some(pending.verifier.clone()),
        refresh_token: None,
        scope: Some(cfg.scope.to_string()),
    };
    let mut resp = state.net.oauth.exchange(req).await?;

    // Keychain first: the token is stored per account under `AiApiKey`, which
    // is exactly where the cloud adapter factories (T059/T060) read it from.
    for account_id in all_account_ids(state.storage.db()).await? {
        let uuid = parse_uuid(&account_id)?;
        state.keychain.set(
            &uuid,
            CredKind::AiApiKey,
            &Secret::new(resp.access_token.as_str()),
        )?;
    }

    apply_recommended_defaults(state, cfg).await?;

    let (ok, model_name, error_message) = probe_with_token(cfg, &resp.access_token).await;

    // Scrub plaintext once Keychain + probe are done (F_A2 §4).
    resp.access_token.zeroize();
    if let Some(rt) = resp.refresh_token.as_mut() {
        rt.zeroize();
    }

    tracing::info!(
        event = "recommended_oauth_complete",
        tier = cfg.tier.as_str(),
        provider = cfg.provider.as_str(),
        verified = ok,
        "recommended-provider oauth grant completed"
    );

    Ok(CompleteRecommendedOAuthResult {
        ok,
        provider_name: cfg.display_name.to_string(),
        model_name: model_name.or_else(|| Some(cfg.model.to_string())),
        error_message,
    })
}

/// Disconnect a recommended tier (F_F3 §4.5): clear the Keychain token and
/// reset every account that was using the tier's provider back to
/// `ai_provider = 'none'`. The configured vendors expose no public revoke
/// endpoint (`revoke_url: None`), so this is a local-only revoke; the UI
/// tells the user they can also revoke from the vendor console.
pub async fn revoke(state: &AppState, tier: RecommendedTier) -> AppResult<()> {
    let cfg = tier_config(tier);
    let db = state.storage.db();
    let now = now_unix();

    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT account_id FROM account_ai_settings WHERE ai_provider = ?")
            .bind(cfg.provider.as_str())
            .fetch_all(db.pool())
            .await
            .map_err(map_sqlx_err)?;

    for (account_id,) in rows {
        if let Ok(uuid) = parse_uuid(&account_id) {
            state.keychain.delete(&uuid, CredKind::AiApiKey)?;
        }
        sqlx::query(
            "UPDATE account_ai_settings SET ai_provider = 'none', ai_model = NULL, \
                 ai_base_url = NULL, ai_api_key_ref = NULL, provider_matrix = NULL, \
                 updated_at = ? \
             WHERE account_id = ?",
        )
        .bind(now)
        .bind(&account_id)
        .execute(db.pool())
        .await
        .map_err(map_sqlx_err)?;
    }

    if cfg.revoke_url.is_some() {
        // No generic HTTP seam exists yet for vendor revocation; when a
        // partner endpoint is confirmed this is where the direct HTTPS call
        // lands. Until then the local clear above is the complete behavior.
        tracing::warn!(
            event = "recommended_revoke_remote_skipped",
            tier = tier.as_str(),
            "vendor revoke endpoint configured but remote revocation is not wired yet"
        );
    }

    tracing::info!(
        event = "recommended_provider_revoked",
        tier = tier.as_str(),
        provider = cfg.provider.as_str(),
        "recommended provider disconnected and accounts reset to none"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ErrorCode;
    use crate::util::new_uuid;

    #[test]
    fn both_tiers_are_configured_and_distinct() {
        let balanced = tier_config(RecommendedTier::Balanced);
        let high = tier_config(RecommendedTier::HighQuality);
        assert_eq!(balanced.tier, RecommendedTier::Balanced);
        assert_eq!(high.tier, RecommendedTier::HighQuality);
        assert_ne!(balanced.provider, high.provider);
        // Both tiers are cloud providers — the disclosure gate must apply.
        assert!(balanced.provider.is_cloud());
        assert!(high.provider.is_cloud());
    }

    #[test]
    fn tier_parse_roundtrip_and_unknown_is_validation() {
        assert_eq!(
            RecommendedTier::parse("balanced").unwrap(),
            RecommendedTier::Balanced
        );
        assert_eq!(
            RecommendedTier::parse("high_quality").unwrap(),
            RecommendedTier::HighQuality
        );
        let err = RecommendedTier::parse("premium").unwrap_err();
        assert_eq!(err.code(), ErrorCode::Validation);
    }

    #[test]
    fn provider_infos_carry_no_endpoints_or_env_names() {
        let infos = recommended_provider_infos();
        assert_eq!(infos.len(), 2);
        let wire = serde_json::to_string(&infos).unwrap();
        assert!(!wire.contains("http"));
        assert!(!wire.contains("SEEKERMAIL_"));
        assert!(wire.contains("monthlyCostMinUsd"));
    }

    #[test]
    fn authorize_url_has_required_pkce_params() {
        let cfg = tier_config(RecommendedTier::Balanced);
        let url = build_authorize_url(cfg, "cid123", "chal", "nonce").unwrap();
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=cid123"));
        assert!(url.contains("code_challenge=chal"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=nonce"));
        assert!(url.contains("redirect_uri=seekermail%3A%2F%2Foauth%2Frecommended"));
    }

    #[test]
    fn callback_parse_extracts_code_and_state() {
        let parsed =
            parse_recommended_callback("seekermail://oauth/recommended?code=abc&state=xyz")
                .unwrap();
        assert_eq!(parsed.code, "abc");
        assert_eq!(parsed.state, "xyz");
        // The account-mail callback path is NOT ours.
        assert!(parse_recommended_callback("seekermail://oauth/callback?code=a&state=b").is_none());
        assert!(parse_recommended_callback("seekermail://oauth/recommended?code=a").is_none());
    }

    #[test]
    fn unknown_state_nonce_is_oauth_failed() {
        let err = take_pending(&new_uuid()).unwrap_err();
        assert_eq!(err.code(), ErrorCode::AuthOauthFailed);
    }

    #[test]
    fn expired_pending_grant_is_oauth_failed() {
        let nonce = new_uuid();
        park_pending(
            &nonce,
            PendingRecommendedAuth {
                tier: RecommendedTier::Balanced,
                verifier: "v".into(),
                created_at: now_unix() - PENDING_TTL_SECS - 10,
            },
        );
        let err = take_pending(&nonce).unwrap_err();
        assert_eq!(err.code(), ErrorCode::AuthOauthFailed);
        // Consumed: a second take is the unknown-state failure, not a replay.
        assert!(take_pending(&nonce).is_err());
    }

    #[tokio::test]
    async fn begin_without_disclosure_is_forbidden() {
        let (state, _rx) = AppState::test_state().await;
        let err = begin(&state, RecommendedTier::Balanced).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::Forbidden);
    }

    #[tokio::test]
    async fn complete_with_wrong_state_is_oauth_failed() {
        let (state, _rx) = AppState::test_state().await;
        let err = complete(&state, "not-a-real-nonce", "code")
            .await
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::AuthOauthFailed);
    }

    #[tokio::test]
    async fn disclosure_confirm_and_status_roundtrip() {
        let (state, _rx) = AppState::test_state().await;
        let db = state.storage.db();

        let before = setup_status(db).await.unwrap();
        assert_eq!(before.disclosure_confirmed_at, None);
        assert_eq!(before.first_auth_at, None);

        let after = confirm_disclosure(db).await.unwrap();
        assert!(after.disclosure_confirmed_at.is_some());

        // Confirmed → the begin gate moves past Forbidden. In a build without
        // the partner client-id env var the next check fails with
        // AUTH_OAUTH_FAILED; with the env var present begin succeeds outright.
        match begin(&state, RecommendedTier::Balanced).await {
            Err(err) => assert_eq!(err.code(), ErrorCode::AuthOauthFailed),
            Ok(begun) => assert!(!begun.state.is_empty()),
        }
    }

    async fn seed_account(state: &AppState, provider: &str) -> String {
        let id = new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
                 created_at, updated_at) VALUES (?, ?, 'Work', 'imap', 'slate', 'W', ?, ?)",
        )
        .bind(&id)
        .bind(format!("{id}@example.com"))
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO account_ai_settings (account_id, auth_level, ai_provider, updated_at) \
             VALUES (?, 1, ?, ?)",
        )
        .bind(&id)
        .bind(provider)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        id
    }

    #[tokio::test]
    async fn apply_defaults_fills_every_account_and_arms_quota_once() {
        let (state, _rx) = AppState::test_state().await;
        let a = seed_account(&state, "none").await;
        let b = seed_account(&state, "none").await;
        let cfg = tier_config(RecommendedTier::Balanced);

        apply_recommended_defaults(&state, cfg).await.unwrap();

        for id in [&a, &b] {
            let (provider, model, key_ref, matrix): (
                String,
                Option<String>,
                Option<String>,
                Option<String>,
            ) = sqlx::query_as(
                "SELECT ai_provider, ai_model, ai_api_key_ref, provider_matrix \
                 FROM account_ai_settings WHERE account_id = ?",
            )
            .bind(id)
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
            assert_eq!(provider, cfg.provider.as_str());
            assert_eq!(model.as_deref(), Some(cfg.model));
            assert_eq!(key_ref.as_deref(), Some(id.as_str()));
            assert!(matrix.is_some(), "F4 defaults must be materialized");
        }

        let status = setup_status(state.storage.db()).await.unwrap();
        let first = status.first_auth_at.expect("first auth stamped");
        let until = status.conservative_quota_until.expect("quota armed");
        assert_eq!(until, first + CONSERVATIVE_QUOTA_SECS);
        assert!(conservative_quota_active(state.storage.db()).await.unwrap());

        // A re-authorization later must not move the first-auth stamp or
        // re-arm an already-lifted quota.
        clear_conservative_quota(state.storage.db()).await.unwrap();
        apply_recommended_defaults(&state, cfg).await.unwrap();
        let status = setup_status(state.storage.db()).await.unwrap();
        assert_eq!(status.first_auth_at, Some(first));
        assert_eq!(status.conservative_quota_until, None);
        assert!(!conservative_quota_active(state.storage.db()).await.unwrap());
    }

    #[tokio::test]
    async fn revoke_resets_matching_accounts_only() {
        let (state, _rx) = AppState::test_state().await;
        let cfg = tier_config(RecommendedTier::Balanced);
        let mine = seed_account(&state, cfg.provider.as_str()).await;
        let other = seed_account(&state, "ollama").await;

        revoke(&state, RecommendedTier::Balanced).await.unwrap();

        let (provider, model, key_ref): (String, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT ai_provider, ai_model, ai_api_key_ref FROM account_ai_settings \
                 WHERE account_id = ?",
        )
        .bind(&mine)
        .fetch_one(state.storage.db().pool())
        .await
        .unwrap();
        assert_eq!(provider, "none");
        assert_eq!(model, None);
        assert_eq!(key_ref, None);

        let (untouched,): (String,) =
            sqlx::query_as("SELECT ai_provider FROM account_ai_settings WHERE account_id = ?")
                .bind(&other)
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        assert_eq!(untouched, "ollama");
    }

    #[tokio::test]
    async fn quota_reader_handles_missing_and_past_values() {
        let (state, _rx) = AppState::test_state().await;
        let db = state.storage.db();
        assert!(!conservative_quota_active(db).await.unwrap());

        SettingRepo::new(db)
            .set(CONSERVATIVE_QUOTA_UNTIL_KEY, &(now_unix() - 10).to_string())
            .await
            .unwrap();
        assert!(!conservative_quota_active(db).await.unwrap());

        SettingRepo::new(db)
            .set(CONSERVATIVE_QUOTA_UNTIL_KEY, &(now_unix() + 60).to_string())
            .await
            .unwrap();
        assert!(conservative_quota_active(db).await.unwrap());
    }
}
