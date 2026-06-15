//! `AiRegistry` — capability × account routing (T058, dev/06 §3, F_F4 §4).
//!
//! Holds the concrete adapter instances (injected at boot by the adapter cards
//! T059–T063) and resolves which one serves a given `(account, capability)`
//! call. The fallback chain is: account-specific override → account default →
//! global default → `none` (F_F4 §4.1–§4.2). T065 added the per-capability
//! override hop: `account_ai_settings.provider_matrix` (a persisted
//! [`CapabilityMatrix`]) is consulted first, and a capability without a matrix
//! entry falls back to the base `ai_provider` columns — the pre-T065 behavior,
//! unchanged. [`AiRegistry::resolve_backup`] walks a cell's backup chain for
//! the T067 offline fallback.
//!
//! ADR-0004 (no proxy): the registry holds **no** SeekerMail server address —
//! it only ever returns adapters pointed at endpoints the user configured.
//!
//! `daily_query_limit` is enforced *here*, before any network call, by counting
//! today's `ai_decisions` rows for the account (dev/06 §3 cost guardrail).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::error::{AppError, AppResult};
use crate::storage::Db;
use crate::types::AiProvider;

use super::matrix::{CapabilityMatrix, ProviderAssignment};
use super::provider::AiProviderClient;
use super::types::Capability;

/// Everything an adapter needs to serve one account (the routing-relevant
/// columns of `account_ai_settings`). Secrets stay in the Keychain — this only
/// carries the *reference* (`api_key_ref`), never key material.
#[derive(Debug, Clone)]
pub struct AccountAiConfig {
    pub account_id: String,
    pub provider: AiProvider,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub api_key_ref: Option<String>,
    pub daily_query_limit: i64,
    /// `account_ai_settings.updated_at` — cache-invalidation stamp.
    pub updated_at: i64,
}

/// Builds a per-account adapter from its config. Cloud adapters are per-account
/// (model/key/endpoint differ per account), so the adapter cards (T059–T063)
/// register factories rather than singletons.
pub type ProviderFactory =
    dyn Fn(&AccountAiConfig) -> AppResult<Arc<dyn AiProviderClient>> + Send + Sync;

/// One factory-built adapter plus the config fingerprint it was built from.
/// Matrix routing (T065) can give the same account different models or
/// endpoints per capability, so cache validity checks the whole effective
/// config, not just the settings stamp.
#[derive(Clone)]
struct CachedClient {
    /// `account_ai_settings.updated_at` at build time. Every settings *and*
    /// matrix write bumps the stamp, so a stale entry can never survive a
    /// config change.
    stamp: i64,
    /// Effective model the adapter was built for (may come from a matrix cell).
    model: Option<String>,
    /// Effective endpoint override the adapter was built for.
    base_url: Option<String>,
    client: Arc<dyn AiProviderClient>,
}

/// Router from `(account, capability)` to a concrete provider client.
/// Cheap to clone — every map is shared behind an `Arc`.
#[derive(Clone)]
pub struct AiRegistry {
    /// Account-agnostic adapters (local singletons, test mocks), keyed by
    /// provider id. Checked only when no factory is registered.
    providers: Arc<RwLock<HashMap<AiProvider, Arc<dyn AiProviderClient>>>>,
    /// Per-account adapter factories (cloud adapters, T059/T060).
    factories: Arc<RwLock<HashMap<AiProvider, Arc<ProviderFactory>>>>,
    /// Built per-account adapters, keyed `(account_id, provider)` so matrix
    /// routing holds one instance per provider per account; entries are
    /// revalidated against the [`CachedClient`] fingerprint on every hit.
    cache: Arc<RwLock<HashMap<(String, AiProvider), CachedClient>>>,
    db: Db,
}

impl AiRegistry {
    pub fn new(db: Db) -> Self {
        Self {
            providers: Arc::new(RwLock::new(HashMap::new())),
            factories: Arc::new(RwLock::new(HashMap::new())),
            cache: Arc::new(RwLock::new(HashMap::new())),
            db,
        }
    }

    /// Register (or replace) an account-agnostic adapter for one provider.
    /// Used for local singletons and test mocks; later registrations win so
    /// tests can swap in mocks.
    pub fn register(&self, client: Arc<dyn AiProviderClient>) {
        let id = client.id();
        self.providers
            .write()
            .expect("ai registry lock poisoned")
            .insert(id, client);
    }

    /// Register the per-account adapter factory for one provider (cloud
    /// adapters: model, endpoint, and key reference differ per account).
    pub fn register_factory(&self, provider: AiProvider, factory: Arc<ProviderFactory>) {
        self.factories
            .write()
            .expect("ai registry lock poisoned")
            .insert(provider, factory);
    }

    /// Which providers currently have a registered adapter or factory (used by
    /// the data-flow disclosure panel, T069, and the config UI).
    pub fn registered(&self) -> Vec<AiProvider> {
        let mut ids: Vec<AiProvider> = self
            .providers
            .read()
            .expect("ai registry lock poisoned")
            .keys()
            .chain(
                self.factories
                    .read()
                    .expect("ai registry lock poisoned")
                    .keys(),
            )
            .copied()
            .collect();
        ids.sort_by_key(|p| p.as_str());
        ids.dedup();
        ids
    }

    /// Resolve which provider handles `(account, capability)` (dev/06 §3).
    ///
    /// Fallback chain: matrix cell for the capability (T065, F_F4 §4.1) →
    /// account default (`account_ai_settings.ai_provider`) → `none`. A matrix
    /// hit routes to the cell's *primary* assignment only — backups are walked
    /// by [`Self::resolve_backup`] after a primary failure (T067). `none` or
    /// an unregistered provider yields `FORBIDDEN` (AI not configured), and an
    /// exhausted `daily_query_limit` yields `AI_RATE_LIMITED` — both *before*
    /// any network call.
    pub async fn resolve(
        &self,
        account_id: &str,
        cap: Capability,
    ) -> AppResult<Arc<dyn AiProviderClient>> {
        let base = self.account_config(account_id).await?;

        // F4 matrix hop: a capability entry overrides provider/model/endpoint
        // for this call; a missing matrix or entry keeps the base config.
        let config = match self
            .provider_matrix(account_id)
            .await?
            .as_ref()
            .and_then(|matrix| matrix.cell(cap))
        {
            Some(cell) => apply_assignment(&base, &cell.primary),
            None => base,
        };

        if config.provider == AiProvider::None {
            return Err(AppError::Forbidden(format!(
                "ai is not configured for this account (capability {})",
                cap.as_str()
            )));
        }

        // Cost guardrail: count today's audit rows before resolving (dev/06 §3).
        // While the first-week conservative quota is armed (T064, F_F3 §4.6)
        // the account's own limit is capped at the conservative ceiling.
        let limit = self.effective_daily_limit(config.daily_query_limit).await?;
        let used = self.decisions_today(account_id).await?;
        if used >= limit {
            tracing::warn!(
                event = "ai_daily_limit_hit",
                account_id = account_id,
                capability = cap.as_str(),
                used = used,
                limit = limit,
                "daily ai query limit reached; refusing before network call"
            );
            return Err(AppError::AiRateLimited);
        }

        self.client_for(&config)
    }

    /// The account's daily limit, capped by the first-week conservative quota
    /// when armed (T064, F_F3 §4.6). A missing or past
    /// `app_settings["ai.conservative_quota_until"]` leaves the limit as-is.
    async fn effective_daily_limit(&self, configured: i64) -> AppResult<i64> {
        if super::recommended::conservative_quota_active(&self.db).await? {
            Ok(configured.min(super::recommended::CONSERVATIVE_DAILY_LIMIT))
        } else {
            Ok(configured)
        }
    }

    /// Clamp a request to the conservative first-week ceiling (T064, F_F3
    /// §4.6): `max_tokens ≤ 2000` while the quota is armed, untouched
    /// otherwise. Invokers (the F5 router / D-E engines) call this right
    /// before `chat()` so the cap applies regardless of who built the request.
    pub async fn clamp_chat_request(
        &self,
        request: &mut super::types::ChatRequest,
    ) -> AppResult<()> {
        if super::recommended::conservative_quota_active(&self.db).await? {
            request.max_tokens = request
                .max_tokens
                .min(super::recommended::CONSERVATIVE_MAX_TOKENS);
        }
        Ok(())
    }

    /// The next usable backup for `(account, capability)` after the primary
    /// (and any providers in `exclude`) failed — the T067 offline-fallback hop
    /// (F_F4 §4.2). Walks the matrix cell's `backups` in order, skipping
    /// excluded, `none`, and unavailable providers; `Ok(None)` when no matrix,
    /// no cell, or no usable link remains. The `daily_query_limit` guardrail
    /// applies here exactly as in [`Self::resolve`].
    pub async fn resolve_backup(
        &self,
        account_id: &str,
        cap: Capability,
        exclude: &[AiProvider],
    ) -> AppResult<Option<Arc<dyn AiProviderClient>>> {
        let base = self.account_config(account_id).await?;

        let limit = self.effective_daily_limit(base.daily_query_limit).await?;
        let used = self.decisions_today(account_id).await?;
        if used >= limit {
            return Err(AppError::AiRateLimited);
        }

        let Some(matrix) = self.provider_matrix(account_id).await? else {
            return Ok(None);
        };
        let Some(cell) = matrix.cell(cap) else {
            return Ok(None);
        };

        for backup in &cell.backups {
            if backup.provider == AiProvider::None || exclude.contains(&backup.provider) {
                continue;
            }
            let config = apply_assignment(&base, backup);
            match self.client_for(&config) {
                Ok(client) => return Ok(Some(client)),
                Err(err) => {
                    // Unregistered in this build / failed to construct → try
                    // the next link rather than failing the whole fallback.
                    tracing::debug!(
                        event = "ai_backup_skipped",
                        account_id = account_id,
                        capability = cap.as_str(),
                        provider = backup.provider.as_str(),
                        error = %err,
                        "backup provider unavailable; trying next link"
                    );
                }
            }
        }
        Ok(None)
    }

    /// Build (or serve from cache) the adapter for one *effective* config.
    /// Factory-built adapters are cached per `(account, provider)` and
    /// revalidated against the stamp + model + endpoint fingerprint;
    /// account-agnostic singletons are returned as-is.
    fn client_for(&self, config: &AccountAiConfig) -> AppResult<Arc<dyn AiProviderClient>> {
        let factory = self
            .factories
            .read()
            .expect("ai registry lock poisoned")
            .get(&config.provider)
            .cloned();
        if let Some(factory) = factory {
            let key = (config.account_id.clone(), config.provider);
            if let Some(cached) = self
                .cache
                .read()
                .expect("ai registry lock poisoned")
                .get(&key)
                .cloned()
            {
                if cached.stamp == config.updated_at
                    && cached.model == config.model
                    && cached.base_url == config.base_url
                {
                    return Ok(cached.client);
                }
            }
            let client = factory(config)?;
            self.cache
                .write()
                .expect("ai registry lock poisoned")
                .insert(
                    key,
                    CachedClient {
                        stamp: config.updated_at,
                        model: config.model.clone(),
                        base_url: config.base_url.clone(),
                        client: client.clone(),
                    },
                );
            return Ok(client);
        }

        // Account-agnostic singleton (local adapters, test mocks).
        let client = self
            .providers
            .read()
            .expect("ai registry lock poisoned")
            .get(&config.provider)
            .cloned();

        client.ok_or_else(|| {
            AppError::Forbidden(format!(
                "provider '{}' is selected but not available in this build",
                config.provider.as_str()
            ))
        })
    }

    /// Load and parse the account's F4 matrix column (T065). `None` when the
    /// column is `NULL` or the account has no settings row. A corrupt payload
    /// is logged and treated as unconfigured — routing degrades to the base
    /// provider columns instead of hard-failing the AI call.
    pub async fn provider_matrix(&self, account_id: &str) -> AppResult<Option<CapabilityMatrix>> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT provider_matrix FROM account_ai_settings WHERE account_id = ?")
                .bind(account_id)
                .fetch_optional(self.db.pool())
                .await
                .map_err(crate::storage::map_sqlx_err)?;
        let Some((Some(json),)) = row else {
            return Ok(None);
        };
        match CapabilityMatrix::from_json(&json) {
            Ok(matrix) => Ok(Some(matrix)),
            Err(err) => {
                tracing::warn!(
                    event = "ai_matrix_parse_failed",
                    account_id = account_id,
                    error = %err,
                    "stored provider matrix is unreadable; falling back to base provider settings"
                );
                Ok(None)
            }
        }
    }

    /// Read the routing-relevant columns of `account_ai_settings`.
    pub async fn account_config(&self, account_id: &str) -> AppResult<AccountAiConfig> {
        /// `(ai_provider, ai_model, ai_base_url, ai_api_key_ref, daily_query_limit, updated_at)`.
        type SettingsRow = (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            i64,
            i64,
        );
        let row: Option<SettingsRow> =
            sqlx::query_as(
                "SELECT ai_provider, ai_model, ai_base_url, ai_api_key_ref, daily_query_limit, updated_at \
                 FROM account_ai_settings WHERE account_id = ?",
            )
            .bind(account_id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(crate::storage::map_sqlx_err)?;

        let (provider, model, base_url, api_key_ref, daily_query_limit, updated_at) =
            row.ok_or(AppError::NotFound)?;
        Ok(AccountAiConfig {
            account_id: account_id.to_string(),
            provider: AiProvider::parse(&provider),
            model,
            base_url,
            api_key_ref,
            daily_query_limit,
            updated_at,
        })
    }

    /// How many `ai_decisions` rows the account has written since 00:00 UTC
    /// today. The audit table is append-only, so this is the authoritative
    /// spend counter (dev/06 §9).
    async fn decisions_today(&self, account_id: &str) -> AppResult<i64> {
        let day_start = today_utc_start();
        let (count,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM ai_decisions WHERE account_id = ? AND created_at >= ?",
        )
        .bind(account_id)
        .bind(day_start)
        .fetch_one(self.db.pool())
        .await
        .map_err(crate::storage::map_sqlx_err)?;
        Ok(count)
    }
}

/// The effective config for one matrix assignment (T065): the cell's
/// provider/model/endpoint layered over the account's base config. Key
/// reference, daily limit, and the cache-invalidation stamp always come from
/// the base row. When the assignment keeps the base provider, an empty model
/// or absent endpoint inherits the base values; when it switches provider,
/// the base model/endpoint do not carry over (they belong to the other
/// provider) — an empty model then means "the provider's default".
fn apply_assignment(base: &AccountAiConfig, assignment: &ProviderAssignment) -> AccountAiConfig {
    let same_provider = assignment.provider == base.provider;
    let model = if assignment.model.trim().is_empty() {
        if same_provider {
            base.model.clone()
        } else {
            None
        }
    } else {
        Some(assignment.model.clone())
    };
    let base_url = assignment.base_url.clone().or_else(|| {
        if same_provider {
            base.base_url.clone()
        } else {
            None
        }
    });
    AccountAiConfig {
        account_id: base.account_id.clone(),
        provider: assignment.provider,
        model,
        base_url,
        api_key_ref: base.api_key_ref.clone(),
        daily_query_limit: base.daily_query_limit,
        updated_at: base.updated_at,
    }
}

/// Unix timestamp of 00:00:00 UTC today.
fn today_utc_start() -> i64 {
    let now = crate::util::now_unix();
    now - now.rem_euclid(86_400)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::matrix::{MatrixCell, MatrixEntry};
    use crate::ai::mock::MockProvider;
    use crate::ai::types::{Capability, ChatRequest};
    use crate::util::{new_uuid, now_unix};

    async fn db() -> Db {
        let db = Db::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    /// Insert a minimal account + its ai-settings row.
    async fn seed_account(db: &Db, provider: &str, daily_limit: i64) -> String {
        let id = new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, created_at, updated_at) \
             VALUES (?, ?, 'Test', 'imap', 'slate', 'W', ?, ?)",
        )
        .bind(&id)
        .bind(format!("{id}@example.com"))
        .bind(now)
        .bind(now)
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO account_ai_settings (account_id, auth_level, ai_provider, daily_query_limit, updated_at) \
             VALUES (?, 1, ?, ?, ?)",
        )
        .bind(&id)
        .bind(provider)
        .bind(daily_limit)
        .bind(now)
        .execute(db.pool())
        .await
        .unwrap();
        id
    }

    /// Persist a matrix for the account, bumping `updated_at` the way the
    /// `update_provider_matrix` command does (T065 §7).
    async fn set_matrix(db: &Db, account_id: &str, matrix: &CapabilityMatrix) {
        sqlx::query(
            "UPDATE account_ai_settings SET provider_matrix = ?, updated_at = updated_at + 1 \
             WHERE account_id = ?",
        )
        .bind(matrix.to_json())
        .bind(account_id)
        .execute(db.pool())
        .await
        .unwrap();
    }

    /// One-entry matrix builder for routing tests.
    fn matrix_with(
        cap: Capability,
        primary: AiProvider,
        backups: &[AiProvider],
    ) -> CapabilityMatrix {
        let assignment = |provider: AiProvider| ProviderAssignment {
            provider,
            model: String::new(),
            base_url: None,
        };
        CapabilityMatrix {
            entries: vec![MatrixEntry {
                capability: cap,
                cell: MatrixCell {
                    primary: assignment(primary),
                    backups: backups.iter().copied().map(assignment).collect(),
                },
            }],
        }
    }

    /// Append one `ai_decisions` audit row at `created_at`.
    async fn seed_decision(db: &Db, account_id: &str, created_at: i64) {
        sqlx::query(
            "INSERT INTO ai_decisions (id, account_id, decision_type, impact, action_description, result_description, created_at) \
             VALUES (?, ?, 'draft_created', 'reply', 'unit-test row', 'ok', ?)",
        )
        .bind(new_uuid())
        .bind(account_id)
        .bind(created_at)
        .execute(db.pool())
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn resolve_none_provider_returns_forbidden() {
        let db = db().await;
        let account = seed_account(&db, "none", 10).await;
        let registry = AiRegistry::new(db);

        let err = registry
            .resolve(&account, Capability::DraftReply)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn resolve_unknown_account_returns_not_found() {
        let db = db().await;
        let registry = AiRegistry::new(db);
        let err = registry
            .resolve("missing-account", Capability::Summarize)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound));
    }

    #[tokio::test]
    async fn resolve_unregistered_provider_returns_forbidden() {
        let db = db().await;
        let account = seed_account(&db, "openai", 10).await;
        let registry = AiRegistry::new(db); // nothing registered

        let err = registry
            .resolve(&account, Capability::DraftReply)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn resolve_registered_provider_returns_instance() {
        let db = db().await;
        let account = seed_account(&db, "openai", 10).await;
        let registry = AiRegistry::new(db);
        registry.register(Arc::new(MockProvider::healthy(AiProvider::Openai)));

        let client = registry
            .resolve(&account, Capability::DraftReply)
            .await
            .unwrap();
        assert_eq!(client.id(), AiProvider::Openai);
    }

    #[tokio::test]
    async fn resolve_daily_limit_exceeded() {
        let db = db().await;
        let account = seed_account(&db, "openai", 2).await;
        let now = now_unix();
        seed_decision(&db, &account, now).await;
        seed_decision(&db, &account, now).await;

        let registry = AiRegistry::new(db);
        registry.register(Arc::new(MockProvider::healthy(AiProvider::Openai)));

        let err = registry
            .resolve(&account, Capability::DraftReply)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::AiRateLimited));
    }

    #[tokio::test]
    async fn yesterdays_decisions_do_not_count_against_today() {
        let db = db().await;
        let account = seed_account(&db, "openai", 1).await;
        // Two rows well before today's UTC midnight.
        let yesterday = today_utc_start() - 3_600;
        seed_decision(&db, &account, yesterday).await;
        seed_decision(&db, &account, yesterday - 100).await;

        let registry = AiRegistry::new(db);
        registry.register(Arc::new(MockProvider::healthy(AiProvider::Openai)));

        assert!(registry
            .resolve(&account, Capability::RiskReason)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn factory_builds_per_account_and_caches_until_settings_change() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let db = db().await;
        let account = seed_account(&db, "openai", 10).await;
        let registry = AiRegistry::new(db.clone());

        let builds = Arc::new(AtomicU32::new(0));
        let builds_in_factory = builds.clone();
        registry.register_factory(
            AiProvider::Openai,
            Arc::new(move |cfg: &AccountAiConfig| {
                builds_in_factory.fetch_add(1, Ordering::SeqCst);
                assert_eq!(cfg.provider, AiProvider::Openai);
                Ok(Arc::new(MockProvider::healthy(AiProvider::Openai))
                    as Arc<dyn AiProviderClient>)
            }),
        );

        registry
            .resolve(&account, Capability::DraftReply)
            .await
            .unwrap();
        registry
            .resolve(&account, Capability::Summarize)
            .await
            .unwrap();
        // Same settings stamp → one build, served from cache afterwards.
        assert_eq!(builds.load(Ordering::SeqCst), 1);

        // Touch the settings row → stamp moves → factory builds again.
        sqlx::query(
            "UPDATE account_ai_settings SET updated_at = updated_at + 1 WHERE account_id = ?",
        )
        .bind(&account)
        .execute(db.pool())
        .await
        .unwrap();
        registry
            .resolve(&account, Capability::DraftReply)
            .await
            .unwrap();
        assert_eq!(builds.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn mock_unreachable_maps_to_ai_unreachable() {
        use crate::ai::provider::ProviderError;
        use crate::types::ErrorCode;

        let db = db().await;
        let account = seed_account(&db, "openai", 10).await;
        let registry = AiRegistry::new(db);
        let mock = MockProvider::healthy(AiProvider::Openai);
        mock.push_chat(Err(ProviderError::Unreachable("dns failure".into())));
        registry.register(Arc::new(mock));

        let client = registry
            .resolve(&account, Capability::DraftReply)
            .await
            .unwrap();
        let provider_err = client
            .chat(ChatRequest::simple("m", "hello", Capability::DraftReply))
            .await
            .unwrap_err();
        let app: AppError = provider_err.into();
        assert_eq!(app.code(), ErrorCode::AiProviderUnreachable);
    }

    // ── First-week conservative quota (T064, F_F3 §4.6) ──────────────────────

    /// Arm (or backdate) `app_settings["ai.conservative_quota_until"]`.
    async fn set_quota_until(db: &Db, until: i64) {
        crate::storage::SettingRepo::new(db)
            .set(
                crate::ai::recommended::CONSERVATIVE_QUOTA_UNTIL_KEY,
                &until.to_string(),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn conservative_quota_caps_daily_limit() {
        let db = db().await;
        // Generous per-account limit; the armed quota must cap it at 100.
        let account = seed_account(&db, "openai", 500).await;
        set_quota_until(&db, now_unix() + 7 * 86_400).await;
        let now = now_unix();
        for _ in 0..100 {
            seed_decision(&db, &account, now).await;
        }

        let registry = AiRegistry::new(db);
        registry.register(Arc::new(MockProvider::healthy(AiProvider::Openai)));

        let err = registry
            .resolve(&account, Capability::DraftReply)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::AiRateLimited));
    }

    #[tokio::test]
    async fn conservative_quota_expired_leaves_limit_untouched() {
        let db = db().await;
        let account = seed_account(&db, "openai", 500).await;
        // A past deadline = the quota was lifted (settings-page path).
        set_quota_until(&db, now_unix() - 86_400).await;
        let now = now_unix();
        for _ in 0..100 {
            seed_decision(&db, &account, now).await;
        }

        let registry = AiRegistry::new(db);
        registry.register(Arc::new(MockProvider::healthy(AiProvider::Openai)));

        assert!(registry
            .resolve(&account, Capability::DraftReply)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn conservative_quota_never_raises_a_stricter_account_limit() {
        let db = db().await;
        // Account limit 2 is stricter than the 100 cap — min() keeps 2.
        let account = seed_account(&db, "openai", 2).await;
        set_quota_until(&db, now_unix() + 7 * 86_400).await;
        let now = now_unix();
        seed_decision(&db, &account, now).await;
        seed_decision(&db, &account, now).await;

        let registry = AiRegistry::new(db);
        registry.register(Arc::new(MockProvider::healthy(AiProvider::Openai)));

        let err = registry
            .resolve(&account, Capability::DraftReply)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::AiRateLimited));
    }

    #[tokio::test]
    async fn conservative_quota_applies_to_backup_resolution() {
        let db = db().await;
        let account = seed_account(&db, "anthropic", 500).await;
        set_quota_until(&db, now_unix() + 7 * 86_400).await;
        let now = now_unix();
        for _ in 0..100 {
            seed_decision(&db, &account, now).await;
        }

        let registry = AiRegistry::new(db.clone());
        registry.register(Arc::new(MockProvider::healthy(AiProvider::Anthropic)));
        registry.register(Arc::new(MockProvider::healthy(AiProvider::Openai)));
        let matrix = matrix_with(
            Capability::DraftReply,
            AiProvider::Anthropic,
            &[AiProvider::Openai],
        );
        set_matrix(&db, &account, &matrix).await;

        let err = registry
            .resolve_backup(&account, Capability::DraftReply, &[])
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::AiRateLimited));
    }

    #[tokio::test]
    async fn clamp_chat_request_caps_max_tokens_only_while_armed() {
        let db = db().await;
        let registry = AiRegistry::new(db.clone());

        let mut req = ChatRequest::simple("m", "hello", Capability::DraftReply);
        req.max_tokens = 4_096;

        // Not armed → untouched.
        registry.clamp_chat_request(&mut req).await.unwrap();
        assert_eq!(req.max_tokens, 4_096);

        // Armed → clamped to 2000; an already-smaller request stays as-is.
        set_quota_until(&db, now_unix() + 86_400).await;
        registry.clamp_chat_request(&mut req).await.unwrap();
        assert_eq!(req.max_tokens, 2_000);
        req.max_tokens = 512;
        registry.clamp_chat_request(&mut req).await.unwrap();
        assert_eq!(req.max_tokens, 512);
    }

    // ── F4 matrix routing (T065) ─────────────────────────────────────────────

    #[tokio::test]
    async fn resolve_uses_matrix() {
        let db = db().await;
        let account = seed_account(&db, "openai", 10).await;
        let registry = AiRegistry::new(db.clone());
        registry.register(Arc::new(MockProvider::healthy(AiProvider::Openai)));
        registry.register(Arc::new(MockProvider::healthy(AiProvider::Anthropic)));

        // Base provider is openai; the matrix routes DraftReply to anthropic.
        let matrix = matrix_with(Capability::DraftReply, AiProvider::Anthropic, &[]);
        set_matrix(&db, &account, &matrix).await;

        let client = registry
            .resolve(&account, Capability::DraftReply)
            .await
            .unwrap();
        assert_eq!(client.id(), AiProvider::Anthropic);
    }

    #[tokio::test]
    async fn resolve_fallback_to_base() {
        let db = db().await;
        let account = seed_account(&db, "openai", 10).await;
        let registry = AiRegistry::new(db.clone());
        registry.register(Arc::new(MockProvider::healthy(AiProvider::Openai)));
        registry.register(Arc::new(MockProvider::healthy(AiProvider::Anthropic)));

        // The matrix has a DraftReply entry but none for Summarize → the base
        // `ai_provider` column wins for Summarize (pre-T065 behavior).
        let matrix = matrix_with(Capability::DraftReply, AiProvider::Anthropic, &[]);
        set_matrix(&db, &account, &matrix).await;

        let client = registry
            .resolve(&account, Capability::Summarize)
            .await
            .unwrap();
        assert_eq!(client.id(), AiProvider::Openai);
    }

    #[tokio::test]
    async fn resolve_backup_excludes() {
        let db = db().await;
        let account = seed_account(&db, "anthropic", 10).await;
        let registry = AiRegistry::new(db.clone());
        registry.register(Arc::new(MockProvider::healthy(AiProvider::Anthropic)));
        registry.register(Arc::new(MockProvider::healthy(AiProvider::Openai)));
        registry.register(Arc::new(MockProvider::healthy(AiProvider::Ollama)));

        let matrix = matrix_with(
            Capability::DraftReply,
            AiProvider::Anthropic,
            &[AiProvider::Openai, AiProvider::Ollama],
        );
        set_matrix(&db, &account, &matrix).await;

        // No exclusions → the first backup wins.
        let first = registry
            .resolve_backup(&account, Capability::DraftReply, &[])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.id(), AiProvider::Openai);

        // Excluding the first backup (already tried by T067) → the second.
        let second = registry
            .resolve_backup(&account, Capability::DraftReply, &[AiProvider::Openai])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.id(), AiProvider::Ollama);

        // Both backups excluded → chain exhausted.
        let none = registry
            .resolve_backup(
                &account,
                Capability::DraftReply,
                &[AiProvider::Openai, AiProvider::Ollama],
            )
            .await
            .unwrap();
        assert!(none.is_none());
    }

    #[tokio::test]
    async fn resolve_backup_without_matrix_or_entry_is_none() {
        let db = db().await;
        let account = seed_account(&db, "openai", 10).await;
        let registry = AiRegistry::new(db.clone());
        registry.register(Arc::new(MockProvider::healthy(AiProvider::Openai)));

        // No matrix at all.
        assert!(registry
            .resolve_backup(&account, Capability::DraftReply, &[])
            .await
            .unwrap()
            .is_none());

        // Matrix present, but no entry for the requested capability.
        let matrix = matrix_with(Capability::DraftReply, AiProvider::Openai, &[]);
        set_matrix(&db, &account, &matrix).await;
        assert!(registry
            .resolve_backup(&account, Capability::Summarize, &[])
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn resolve_backup_skips_unregistered_providers() {
        let db = db().await;
        let account = seed_account(&db, "anthropic", 10).await;
        let registry = AiRegistry::new(db.clone());
        registry.register(Arc::new(MockProvider::healthy(AiProvider::Anthropic)));
        // Openai is in the chain but NOT registered; Ollama is.
        registry.register(Arc::new(MockProvider::healthy(AiProvider::Ollama)));

        let matrix = matrix_with(
            Capability::DraftReply,
            AiProvider::Anthropic,
            &[AiProvider::Openai, AiProvider::Ollama],
        );
        set_matrix(&db, &account, &matrix).await;

        let client = registry
            .resolve_backup(&account, Capability::DraftReply, &[])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(client.id(), AiProvider::Ollama);
    }

    #[tokio::test]
    async fn corrupt_matrix_falls_back_to_base_provider() {
        let db = db().await;
        let account = seed_account(&db, "openai", 10).await;
        sqlx::query(
            "UPDATE account_ai_settings SET provider_matrix = '{broken' WHERE account_id = ?",
        )
        .bind(&account)
        .execute(db.pool())
        .await
        .unwrap();

        let registry = AiRegistry::new(db);
        registry.register(Arc::new(MockProvider::healthy(AiProvider::Openai)));

        // Unreadable matrix must never brick AI routing — base column wins.
        let client = registry
            .resolve(&account, Capability::DraftReply)
            .await
            .unwrap();
        assert_eq!(client.id(), AiProvider::Openai);
    }

    #[tokio::test]
    async fn matrix_cell_overrides_model_for_factory_builds() {
        let db = db().await;
        let account = seed_account(&db, "openai", 10).await;
        let registry = AiRegistry::new(db.clone());

        let seen_models = Arc::new(std::sync::Mutex::new(Vec::<Option<String>>::new()));
        let seen = seen_models.clone();
        registry.register_factory(
            AiProvider::Openai,
            Arc::new(move |cfg: &AccountAiConfig| {
                seen.lock().unwrap().push(cfg.model.clone());
                Ok(Arc::new(MockProvider::healthy(AiProvider::Openai))
                    as Arc<dyn AiProviderClient>)
            }),
        );

        // Base resolve first (model = None from the seeded row).
        registry
            .resolve(&account, Capability::DraftReply)
            .await
            .unwrap();

        // Matrix assigns a capability-specific model for the same provider →
        // the factory must be re-invoked with the cell's model.
        let mut matrix = matrix_with(Capability::DraftReply, AiProvider::Openai, &[]);
        matrix.entries[0].cell.primary.model = "gpt-4o-mini".into();
        set_matrix(&db, &account, &matrix).await;
        registry
            .resolve(&account, Capability::DraftReply)
            .await
            .unwrap();

        let models = seen_models.lock().unwrap().clone();
        assert_eq!(models, vec![None, Some("gpt-4o-mini".to_string())]);
    }
}
