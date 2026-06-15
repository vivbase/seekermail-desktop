//! Module F commands — per-account AI settings + provider verification (T059,
//! 02 §Module H).
//!
//! Boundary rules upheld here (ADR-0004, 09 §5):
//!
//! * `update_account_ai_settings` consumes `aiApiKey` at this boundary and
//!   hands it to the OS Keychain; the DB column `ai_api_key_ref` stores only
//!   the Keychain item reference (the account id), never key material, and the
//!   returned [`AccountAiSettings`] is key-free by construction.
//! * `verify_ai_provider` is an in-band probe (09 §2): failures come back as
//!   `Ok` with `ok = false` plus the sanitized, content-free `ProviderError`
//!   rendering. The transient key lives only inside the command frame.

use tauri::State;

use crate::ai::audit::types::{
    AiDecisionRow, DecisionSummary, ExportAiDecisionsParams, ListDecisionsParams,
};
use crate::ai::audit::{decision_type, AuditEntry};
use crate::ai::draft::repo as ai_draft_repo;
use crate::ai::matrix::{build_default_matrix, BatchMatrixUpdate, CapabilityMatrix, MatrixWarning};
use crate::ai::providers;
use crate::error::{AppError, AppResult, IpcError};
use crate::keychain::{CredKind, Secret};
use crate::state::AppState;
use crate::storage::map_sqlx_err;
use crate::types::{
    AccountAiSettings, AiDraft, AiProvider, ApproveDraftResult, ConfiguredProviderInfo,
    ListAiDraftsParams, LocalProviderEndpoint, OllamaModelEntry, RegenerateDraftParams,
    RequestAiReplyParams, SendMailParams, UpdateAiSettingsParams, VerifyAiProviderParams,
    VerifyAiProviderResult,
};
use crate::util::{now_unix, parse_uuid};

/// DB projection of an `account_ai_settings` row (the columns the
/// [`AccountAiSettings`] DTO needs — `ai_api_key_ref` is deliberately absent).
#[derive(sqlx::FromRow)]
struct AiSettingsRow {
    account_id: String,
    auth_level: i64,
    ai_provider: String,
    ai_model: Option<String>,
    ai_base_url: Option<String>,
    t1_enabled: i64,
    t2_enabled: i64,
    t3_enabled: i64,
    t4_enabled: i64,
    t5_enabled: i64,
    t6_enabled: i64,
    daily_query_limit: i64,
    e3_whitelist_only: i64,
    e3_min_history: i64,
    style_samples_count: i64,
    updated_at: i64,
}

impl From<AiSettingsRow> for AccountAiSettings {
    fn from(r: AiSettingsRow) -> Self {
        AccountAiSettings {
            account_id: r.account_id,
            auth_level: r.auth_level as u8,
            ai_provider: AiProvider::parse(&r.ai_provider),
            ai_model: r.ai_model,
            ai_base_url: r.ai_base_url,
            t1_enabled: r.t1_enabled != 0,
            t2_enabled: r.t2_enabled != 0,
            t3_enabled: r.t3_enabled != 0,
            t4_enabled: r.t4_enabled != 0,
            t5_enabled: r.t5_enabled != 0,
            t6_enabled: r.t6_enabled != 0,
            daily_query_limit: r.daily_query_limit.max(0) as u32,
            e3_whitelist_only: r.e3_whitelist_only != 0,
            e3_min_history: r.e3_min_history.max(0) as u32,
            style_samples_count: r.style_samples_count.max(0) as u32,
            updated_at: r.updated_at,
        }
    }
}

const SELECT_COLS: &str = "account_id, auth_level, ai_provider, ai_model, ai_base_url, \
     t1_enabled, t2_enabled, t3_enabled, t4_enabled, t5_enabled, t6_enabled, \
     daily_query_limit, e3_whitelist_only, e3_min_history, style_samples_count, updated_at";

/// Read one settings row, `NOT_FOUND` when the account has none.
async fn fetch_settings(state: &AppState, account_id: &str) -> AppResult<AccountAiSettings> {
    let sql = format!("SELECT {SELECT_COLS} FROM account_ai_settings WHERE account_id = ?");
    let row: Option<AiSettingsRow> = sqlx::query_as(&sql)
        .bind(account_id)
        .fetch_optional(state.storage.db().pool())
        .await
        .map_err(map_sqlx_err)?;
    row.map(AccountAiSettings::from).ok_or(AppError::NotFound)
}

/// Partial update (read-modify-write, same pattern as `AccountRepo::update`).
/// The inbound key, when present, is written to the Keychain inside this frame
/// and only the item reference (the account id string) reaches the DB.
async fn do_update(
    state: &AppState,
    account_id: &str,
    params: UpdateAiSettingsParams,
) -> AppResult<AccountAiSettings> {
    let cur = fetch_settings(state, account_id).await?;

    if let Some(limit) = params.daily_query_limit {
        if limit == 0 {
            return Err(AppError::Validation(
                "dailyQueryLimit must be at least 1".into(),
            ));
        }
    }

    if let Some(level) = params.auth_level {
        if !(1..=3).contains(&level) {
            return Err(AppError::Validation(
                "authLevel must be 1 (manual), 2 (semi-auto), or 3 (full-auto)".into(),
            ));
        }
    }

    // Key write path: Keychain only; the DB stores the item reference.
    let mut api_key_ref: Option<String> = None;
    if let Some(key) = params.ai_api_key.as_deref() {
        if key.trim().is_empty() {
            return Err(AppError::Validation("aiApiKey must not be empty".into()));
        }
        let uuid = parse_uuid(account_id)?;
        state
            .keychain
            .set(&uuid, CredKind::AiApiKey, &Secret::new(key))?;
        api_key_ref = Some(account_id.to_string());
    }

    let auth_level = params.auth_level.unwrap_or(cur.auth_level);
    let ai_provider = params.ai_provider.unwrap_or(cur.ai_provider);
    let ai_model = params.ai_model.clone().or(cur.ai_model);
    let ai_base_url = params.ai_base_url.clone().or(cur.ai_base_url);
    let t1 = params.t1_enabled.unwrap_or(cur.t1_enabled);
    let t2 = params.t2_enabled.unwrap_or(cur.t2_enabled);
    let t3 = params.t3_enabled.unwrap_or(cur.t3_enabled);
    let t4 = params.t4_enabled.unwrap_or(cur.t4_enabled);
    let t5 = params.t5_enabled.unwrap_or(cur.t5_enabled);
    let t6 = params.t6_enabled.unwrap_or(cur.t6_enabled);
    let daily_query_limit = params.daily_query_limit.unwrap_or(cur.daily_query_limit);
    let e3_whitelist_only = params.e3_whitelist_only.unwrap_or(cur.e3_whitelist_only);
    let e3_min_history = params.e3_min_history.unwrap_or(cur.e3_min_history);
    let now = now_unix();

    // E3 enablement gate (T087 §3, F_E3 §4.1): raising the level to Full Auto
    // requires a track record of human-approved drafts. The check runs at this
    // IPC layer only — pipelines trust the stored level afterwards.
    if params.auth_level == Some(3) {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM ai_decisions \
             WHERE account_id = ? AND decision_type = 'draft_sent'",
        )
        .bind(account_id)
        .fetch_one(state.storage.db().pool())
        .await
        .map_err(map_sqlx_err)?;
        let min = i64::from(e3_min_history);
        if count < min {
            return Err(AppError::Validation(format!(
                "E3 requires at least {min} approved drafts; only {count} found"
            )));
        }
    }

    // `accounts.auth_level` is authoritative; `account_ai_settings.auth_level`
    // mirrors it. A level change updates both rows in one transaction so the
    // pair can never diverge (T087 §6, dev/01 §account_ai_settings).
    let mut tx = state
        .storage
        .db()
        .pool()
        .begin()
        .await
        .map_err(map_sqlx_err)?;
    sqlx::query(
        "UPDATE account_ai_settings SET auth_level = ?, ai_provider = ?, ai_model = ?, \
             ai_base_url = ?, ai_api_key_ref = COALESCE(?, ai_api_key_ref), \
             t1_enabled = ?, t2_enabled = ?, t3_enabled = ?, t4_enabled = ?, \
             t5_enabled = ?, t6_enabled = ?, daily_query_limit = ?, \
             e3_whitelist_only = ?, e3_min_history = ?, updated_at = ? \
         WHERE account_id = ?",
    )
    .bind(auth_level as i64)
    .bind(ai_provider.as_str())
    .bind(&ai_model)
    .bind(&ai_base_url)
    .bind(&api_key_ref)
    .bind(t1 as i64)
    .bind(t2 as i64)
    .bind(t3 as i64)
    .bind(t4 as i64)
    .bind(t5 as i64)
    .bind(t6 as i64)
    .bind(daily_query_limit as i64)
    .bind(e3_whitelist_only as i64)
    .bind(e3_min_history as i64)
    .bind(now)
    .bind(account_id)
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx_err)?;
    if auth_level != cur.auth_level {
        sqlx::query("UPDATE accounts SET auth_level = ?, updated_at = ? WHERE id = ?")
            .bind(auth_level as i64)
            .bind(now)
            .bind(account_id)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_err)?;
    }
    tx.commit().await.map_err(map_sqlx_err)?;

    // Identifiers and flags only — never key material or models' output (09 §5).
    tracing::info!(
        event = "ai_settings_updated",
        account_id = account_id,
        provider = ai_provider.as_str(),
        key_written = api_key_ref.is_some(),
        "account ai settings updated"
    );

    fetch_settings(state, account_id).await
}

/// In-band provider probe (09 §2): every outcome is an `Ok` result; only the
/// sanitized `ProviderError` rendering reaches `error_message`.
async fn do_verify(params: VerifyAiProviderParams) -> VerifyAiProviderResult {
    if params.model.trim().is_empty() {
        return VerifyAiProviderResult {
            ok: false,
            model_name: None,
            error_message: Some("a model name is required".into()),
        };
    }

    // Cross-adapter convention (T059/T060/T062): each adapter module exposes
    // `probe(model, api_key, base_url)` with this exact signature.
    let outcome = match params.provider {
        AiProvider::Openai => {
            providers::openai::probe(
                &params.model,
                params.api_key.as_deref(),
                params.base_url.as_deref(),
            )
            .await
        }
        AiProvider::Anthropic => {
            providers::anthropic::AnthropicClient::probe(
                &params.model,
                params.api_key.as_deref(),
                params.base_url.as_deref(),
            )
            .await
        }
        AiProvider::Ollama => {
            providers::ollama::OllamaClient::probe(
                &params.model,
                params.api_key.as_deref(),
                params.base_url.as_deref(),
            )
            .await
        }
        AiProvider::LocalOnnx | AiProvider::None => {
            return VerifyAiProviderResult {
                ok: false,
                model_name: None,
                error_message: Some("this provider cannot be verified over the network".into()),
            };
        }
    };

    match outcome {
        Ok(health) => VerifyAiProviderResult {
            ok: health.ok,
            model_name: health.model_name,
            error_message: None,
        },
        // `ProviderError` `Display` is content-free by contract (provider.rs):
        // endpoint kind / status only — never prompt, completion, or key text.
        Err(err) => VerifyAiProviderResult {
            ok: false,
            model_name: None,
            error_message: Some(err.to_string()),
        },
    }
}

// ── F4 provider matrix (T065) ────────────────────────────────────────────────

/// Read the persisted matrix; a `NULL` (or unreadable) column yields the
/// computed default for the account's current base provider *without*
/// persisting it (F_F4 §4.1 — defaults materialize only on first save).
async fn do_get_matrix(state: &AppState, account_id: &str) -> AppResult<CapabilityMatrix> {
    let base = state.ai.account_config(account_id).await?;
    if let Some(matrix) = state.ai.provider_matrix(account_id).await? {
        return Ok(matrix);
    }
    Ok(build_default_matrix(&base, &state.ai.registered()))
}

/// Persist one account's matrix. The write bumps `updated_at` so the
/// registry's stamp-based adapter cache invalidates and the next `resolve()`
/// sees the new routing (T065 §7).
async fn write_matrix(
    state: &AppState,
    account_id: &str,
    matrix: &CapabilityMatrix,
) -> AppResult<()> {
    let result = sqlx::query(
        "UPDATE account_ai_settings SET provider_matrix = ?, updated_at = ? \
         WHERE account_id = ?",
    )
    .bind(matrix.to_json())
    .bind(now_unix())
    .bind(account_id)
    .execute(state.storage.db().pool())
    .await
    .map_err(map_sqlx_err)?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    // Identifiers and counts only — never model output or mail content (09 §5).
    tracing::info!(
        event = "ai_matrix_updated",
        account_id = account_id,
        entries = matrix.entries.len(),
        "provider matrix persisted"
    );
    Ok(())
}

/// Validate, persist, and collect the advisory warnings (F_F4 §4.2, §4.5).
/// Structural violations (backup chain > 2, backup repeating the primary
/// provider) reject the save; warnings never do.
async fn do_update_matrix(
    state: &AppState,
    account_id: &str,
    matrix: &CapabilityMatrix,
) -> AppResult<Vec<MatrixWarning>> {
    matrix.validate()?;
    write_matrix(state, account_id, matrix).await?;
    Ok(matrix.warnings())
}

/// Rebuild the default matrix from the account's base provider + the adapters
/// registered in this build, persist it, and return it (F_F4 §4.1
/// "one-click reset"). Deterministic, so repeated resets are idempotent.
async fn do_reset_matrix(state: &AppState, account_id: &str) -> AppResult<CapabilityMatrix> {
    let base = state.ai.account_config(account_id).await?;
    let matrix = build_default_matrix(&base, &state.ai.registered());
    write_matrix(state, account_id, &matrix).await?;
    Ok(matrix)
}

/// Apply a set of `(account, capability) → cell` updates (the T066 batch
/// operations: copy row / copy column / switch-all-E4-to-local, F_F4 §4.3).
/// Every cell is validated up front so one bad item rejects the whole batch
/// before anything is written. Each account's update merges into its
/// *effective* matrix (stored, or the computed default when unset) so a
/// first batch edit materializes the defaults instead of dropping them.
async fn do_batch_update_matrix(state: &AppState, updates: &[BatchMatrixUpdate]) -> AppResult<()> {
    for update in updates {
        update.cell.validate()?;
    }
    for update in updates {
        let mut matrix = do_get_matrix(state, &update.account_id).await?;
        matrix.set_cell(update.capability, update.cell.clone());
        matrix.validate()?;
        write_matrix(state, &update.account_id, &matrix).await?;
    }
    Ok(())
}

// ── F5 user-forced degradation (T067, F_F5 §4.5) ─────────────────────────────

/// Write (or clear) the user-forced AI disable deadline. While
/// `app_settings["ai.disable_until"]` holds a future timestamp, every
/// `FallbackRouter::invoke()` returns `DowngradeDecision { reason:
/// "user_disabled" }` without touching any provider.
async fn do_set_ai_disabled(state: &AppState, until: Option<i64>) -> AppResult<()> {
    let repo = crate::storage::SettingRepo::new(state.storage.db());
    match until {
        Some(ts) => {
            if ts < 0 {
                return Err(AppError::Validation(
                    "until must be a non-negative unix timestamp".into(),
                ));
            }
            repo.set(crate::ai::fallback::AI_DISABLE_UNTIL_KEY, &ts.to_string())
                .await?;
        }
        // `null` = restore AI immediately (card §3).
        None => {
            repo.delete(crate::ai::fallback::AI_DISABLE_UNTIL_KEY)
                .await?;
        }
    }
    tracing::info!(
        event = "ai_disable_updated",
        until = until,
        "user-forced ai degradation window updated"
    );
    Ok(())
}

// ── Provider config UI aggregations (T068, F_F1 §5, F_F2 §3) ─────────────────

/// Map the scanner's reachable base URLs onto the wire DTO. v0.5 scans the
/// default Ollama ports only, so every hit is an Ollama endpoint.
fn to_local_endpoints(bases: Vec<String>) -> Vec<LocalProviderEndpoint> {
    bases
        .into_iter()
        .map(|base_url| LocalProviderEndpoint {
            base_url,
            provider: AiProvider::Ollama,
        })
        .collect()
}

/// Read the daemon's installed models and project them onto the wire shape
/// (F_F2 §4.3). Unlike `verify_ai_provider`, failures here are out-of-band:
/// an unreachable daemon surfaces as `AI_PROVIDER_UNREACHABLE`.
async fn do_list_ollama_models(base_url: Option<&str>) -> AppResult<Vec<OllamaModelEntry>> {
    let models = providers::ollama::discover_ollama_models(base_url)
        .await
        .map_err(AppError::from)?;
    Ok(models
        .into_iter()
        .map(|m| OllamaModelEntry {
            name: m.name,
            size_bytes: m.size_bytes,
            parameter_size: m.parameter_size,
            quantization: m.quantization,
        })
        .collect())
}

/// Join projection for the configured-provider aggregation
/// (`accounts ⋈ account_ai_settings`) — key-reference column deliberately absent.
#[derive(sqlx::FromRow)]
struct ConfiguredProviderRow {
    account_id: String,
    email: String,
    display_name: String,
    color_token: String,
    ai_provider: String,
    ai_model: Option<String>,
    ai_base_url: Option<String>,
    auth_level: i64,
    updated_at: i64,
}

/// Every account with a configured (non-`none`) provider, in account-creation
/// order (T068 §3). `available` reflects the adapters registered in this build
/// so the list can flag a provider the current binary cannot serve.
async fn do_list_configured_providers(state: &AppState) -> AppResult<Vec<ConfiguredProviderInfo>> {
    let rows: Vec<ConfiguredProviderRow> = sqlx::query_as(
        "SELECT a.id AS account_id, a.email, a.display_name, a.color_token, \
                s.ai_provider, s.ai_model, s.ai_base_url, s.auth_level, s.updated_at \
         FROM accounts a \
         JOIN account_ai_settings s ON s.account_id = a.id \
         WHERE s.ai_provider != 'none' \
         ORDER BY a.created_at, a.id",
    )
    .fetch_all(state.storage.db().pool())
    .await
    .map_err(map_sqlx_err)?;

    let registered = state.ai.registered();
    Ok(rows
        .into_iter()
        .map(|r| {
            let provider = AiProvider::parse(&r.ai_provider);
            ConfiguredProviderInfo {
                account_id: r.account_id,
                email: r.email,
                display_name: r.display_name,
                color_token: r.color_token,
                provider,
                model: r.ai_model,
                base_url: r.ai_base_url,
                auth_level: r.auth_level.clamp(0, i64::from(u8::MAX)) as u8,
                is_local: matches!(provider, AiProvider::Ollama | AiProvider::LocalOnnx),
                available: registered.contains(&provider),
                updated_at: r.updated_at,
            }
        })
        .collect())
}

// ── Commands (02 §Module H) ──────────────────────────────────────────────────

/// The account's AI settings row. Errors: `NOT_FOUND`.
#[tauri::command]
pub async fn get_account_ai_settings(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<AccountAiSettings, IpcError> {
    fetch_settings(&state, &account_id)
        .await
        .map_err(IpcError::from)
}

/// Partial update of the account's AI settings. `aiApiKey` goes to the
/// Keychain; the response never carries key material. Errors: `NOT_FOUND`,
/// `VALIDATION`, `AUTH_KEYCHAIN_DENIED`.
#[tauri::command]
pub async fn update_account_ai_settings(
    state: State<'_, AppState>,
    account_id: String,
    params: UpdateAiSettingsParams,
) -> Result<AccountAiSettings, IpcError> {
    do_update(&state, &account_id, params)
        .await
        .map_err(IpcError::from)
}

/// Test an AI provider key/endpoint without saving (02 §Module H). In-band
/// result: probe failures return `Ok` with `ok = false` (09 §2).
#[tauri::command]
pub async fn verify_ai_provider(
    params: VerifyAiProviderParams,
) -> Result<VerifyAiProviderResult, IpcError> {
    Ok(do_verify(params).await)
}

/// The account's F4 capability × provider matrix (T065). A `NULL` column
/// returns the computed defaults without persisting them. Errors: `NOT_FOUND`.
#[tauri::command]
pub async fn get_provider_matrix(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<CapabilityMatrix, IpcError> {
    do_get_matrix(&state, &account_id)
        .await
        .map_err(IpcError::from)
}

/// Replace the account's matrix. Returns the non-blocking advisory warnings
/// (F_F4 §4.5) — the save has already succeeded when they arrive. Errors:
/// `NOT_FOUND`, `VALIDATION` (backup chain > 2, backup repeats the primary).
#[tauri::command]
pub async fn update_provider_matrix(
    state: State<'_, AppState>,
    account_id: String,
    matrix: CapabilityMatrix,
) -> Result<Vec<MatrixWarning>, IpcError> {
    do_update_matrix(&state, &account_id, &matrix)
        .await
        .map_err(IpcError::from)
}

/// Reset the account's matrix to the computed defaults and return the new
/// matrix (F_F4 §4.1). Errors: `NOT_FOUND`.
#[tauri::command]
pub async fn reset_provider_matrix_to_defaults(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<CapabilityMatrix, IpcError> {
    do_reset_matrix(&state, &account_id)
        .await
        .map_err(IpcError::from)
}

/// Apply a batch of cell updates across accounts/capabilities (F_F4 §4.3,
/// consumed by the T066 batch operations). All cells are validated before any
/// write. Errors: `NOT_FOUND`, `VALIDATION`.
#[tauri::command]
pub async fn batch_update_provider_matrix(
    state: State<'_, AppState>,
    updates: Vec<BatchMatrixUpdate>,
) -> Result<(), IpcError> {
    do_batch_update_matrix(&state, &updates)
        .await
        .map_err(IpcError::from)
}

/// Temporarily disable all AI capabilities until `until` (unix seconds), or
/// restore them with `null` (T067, F_F5 §4.5 — the 24 h / 48 h / permanent
/// switch in settings). Errors: `VALIDATION`.
#[tauri::command]
pub async fn set_ai_disabled(
    state: State<'_, AppState>,
    until: Option<i64>,
) -> Result<(), IpcError> {
    do_set_ai_disabled(&state, until)
        .await
        .map_err(IpcError::from)
}

/// Probe the default local AI endpoints and return the reachable ones in probe
/// order (T068, F_F2 §3). An empty list is the in-band "nothing found" outcome
/// — the config UI then offers manual entry. Never errors.
#[tauri::command]
pub async fn scan_local_providers() -> Result<Vec<LocalProviderEndpoint>, IpcError> {
    Ok(to_local_endpoints(
        providers::ollama::scan_default_ollama_endpoints().await,
    ))
}

/// Models installed on an Ollama daemon (T068, F_F2 §4.3). `base_url = null`
/// probes the default endpoint. Errors: `AI_PROVIDER_UNREACHABLE`.
#[tauri::command]
pub async fn list_ollama_models(
    base_url: Option<String>,
) -> Result<Vec<OllamaModelEntry>, IpcError> {
    do_list_ollama_models(base_url.as_deref())
        .await
        .map_err(IpcError::from)
}

/// Provider summary across all accounts — one row per account whose
/// `ai_provider` is not `none` (T068 §3; the Settings → AI Providers list and
/// the T066 matrix UI both read this).
#[tauri::command]
pub async fn list_configured_providers(
    state: State<'_, AppState>,
) -> Result<Vec<ConfiguredProviderInfo>, IpcError> {
    do_list_configured_providers(&state)
        .await
        .map_err(IpcError::from)
}

// ── Module E — E1 manual reply generation (T077, 02 §Module E) ──────────────

/// Generate an E1 manual reply draft for one mail (F_E1 §4). Works at every
/// authorization level — the trigger is an explicit user action. Errors:
/// `NOT_FOUND`, `AI_PROVIDER_UNREACHABLE`, `AI_CONTEXT_TOO_LONG`,
/// `AI_RATE_LIMITED`.
#[tauri::command]
pub async fn request_ai_reply(
    state: State<'_, AppState>,
    params: RequestAiReplyParams,
) -> Result<AiDraft, IpcError> {
    crate::ai::draft::engine::generate_e1(&state, &params.mail_id, params.instruction.as_deref())
        .await
        .map_err(IpcError::from)
}

/// Regenerate a draft (F_E1 §4.6): a fresh draft is generated for the same
/// trigger mail and the old one becomes `discarded`/`superseded`. Errors:
/// `NOT_FOUND`, `AI_PROVIDER_UNREACHABLE`, `AI_CONTEXT_TOO_LONG`,
/// `AI_RATE_LIMITED`.
#[tauri::command]
pub async fn regenerate_draft(
    state: State<'_, AppState>,
    params: RegenerateDraftParams,
) -> Result<AiDraft, IpcError> {
    crate::ai::draft::engine::regenerate(&state, &params.id, params.instruction.as_deref())
        .await
        .map_err(IpcError::from)
}

// ── E6 draft queue lifecycle (T080) + approve/cancel send (T090) ─────────────
//
// Wire-code note: dev/02 specifies CONFLICT for state-machine violations
// (double-approve, cancel-after-send). This codebase's `ErrorCode` set (T004)
// has no CONFLICT — `FORBIDDEN` is the established code for "the current
// state does not allow this action" and is used here for both guards.

/// Default page size for the E6 pending queue.
const PENDING_DRAFTS_DEFAULT_LIMIT: i64 = 50;
/// Hard cap on the pending queue page size.
const PENDING_DRAFTS_MAX_LIMIT: i64 = 200;

/// Write a draft-lifecycle audit record; failures are `warn`-only so the
/// user-visible action never fails on a logging hiccup (F_E7 §7).
async fn audit_draft_event(
    state: &AppState,
    draft: &AiDraft,
    decision: &str,
    action: &str,
    result: &str,
) {
    let entry = AuditEntry {
        account_id: draft.account_id.clone(),
        mail_id: Some(draft.trigger_mail_id.clone()),
        draft_id: Some(draft.id.clone()),
        decision_type: decision.to_string(),
        impact: "reply".into(),
        action_description: action.to_string(),
        result_description: result.to_string(),
        knowledge_refs: Vec::new(),
        knowledge_summary: None,
        ai_model: Some(draft.ai_model.clone()),
        input_tokens: None,
        output_tokens: None,
        latency_ms: None,
    };
    if let Err(e) = state.audit.log_await(entry).await {
        tracing::warn!(
            event = "draft_audit_write_failed",
            code = e.code().as_wire(),
            draft_id = %draft.id,
            decision_type = decision,
            "draft lifecycle audit write failed"
        );
    }
}

/// Apply a user edit to a draft body, audit it, and notify the UI.
async fn do_update_draft_body(state: &AppState, id: &str, body: &str) -> AppResult<AiDraft> {
    let updated = ai_draft_repo::update_body(state.storage.db(), id, body).await?;
    audit_draft_event(
        state,
        &updated,
        decision_type::DRAFT_EDITED,
        "User edited the AI draft body in the review panel.",
        "Draft body updated; status moved to edited.",
    )
    .await;
    state.events.draft_updated(id);
    Ok(updated)
}

/// Discard a draft from the review queue, audit it, and notify the UI.
async fn do_discard_draft(state: &AppState, id: &str, reason: Option<&str>) -> AppResult<()> {
    let draft = ai_draft_repo::get(state.storage.db(), id).await?;
    if draft.status == "sent" {
        return Err(AppError::Forbidden(
            "a sent draft can no longer be discarded".into(),
        ));
    }
    let reason = reason.filter(|r| !r.trim().is_empty()).unwrap_or("user");
    ai_draft_repo::mark_discarded(state.storage.db(), id, reason).await?;
    audit_draft_event(
        state,
        &draft,
        decision_type::DRAFT_DISCARDED,
        "User discarded the AI draft from the review queue.",
        "Draft marked discarded; no reply was sent.",
    )
    .await;
    state.events.draft_discarded(id, Some(reason));
    tracing::info!(
        event = "draft_discarded",
        draft_id = %id,
        reason = reason,
        "ai draft discarded"
    );
    Ok(())
}

/// Approve a draft and send it (T090, F_E6 §4.3).
///
/// The send goes through the existing `send::schedule_send` service rather
/// than a raw SMTP call: that keeps the 10 s cancel window and the offline
/// transport seam, and the returned `pendingId` keeps the frontend's
/// `cancel_send(pendingId)` backstop working. The draft is marked `sent` and
/// the `draft_sent` audit record is written in ONE transaction immediately
/// after scheduling; a scheduling failure (inactive account, validation)
/// propagates with the draft untouched.
async fn do_approve_draft(state: &AppState, id: &str) -> AppResult<ApproveDraftResult> {
    let pool = state.storage.db().pool();
    let draft = ai_draft_repo::get(state.storage.db(), id).await?;
    if !matches!(draft.status.as_str(), "pending" | "edited") {
        return Err(AppError::Forbidden(
            "draft is not in an approvable state".into(),
        ));
    }

    // Threading headers from the trigger mail (reply semantics, RFC 2822).
    let trigger: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT message_id, \"references\" FROM mails WHERE id = ?")
            .bind(&draft.trigger_mail_id)
            .fetch_optional(pool)
            .await
            .map_err(map_sqlx_err)?;
    let (in_reply_to, references) = match trigger {
        Some((message_id, refs)) => {
            let chain = match refs.filter(|r| !r.trim().is_empty()) {
                Some(r) => format!("{r} {message_id}"),
                None => message_id.clone(),
            };
            (Some(message_id), Some(chain))
        }
        None => (None, None),
    };

    let started = std::time::Instant::now();
    let send = crate::send::schedule_send(
        state,
        SendMailParams {
            account_id: draft.account_id.clone(),
            to: vec![draft.to_addr.clone()],
            cc: draft.cc_addrs.clone(),
            bcc: Vec::new(),
            subject: draft.subject.clone(),
            body_text: draft.body_current.clone(),
            body_html: None,
            in_reply_to,
            references,
            draft_id: None,
        },
    )
    .await?;

    // Status flip + audit record commit atomically (T090 §6).
    let sent_at = now_unix();
    let mut tx = pool.begin().await.map_err(map_sqlx_err)?;
    ai_draft_repo::mark_sent_tx(&mut tx, id, sent_at).await?;
    crate::ai::audit::repo::insert_decision_tx(
        &mut tx,
        &AuditEntry {
            account_id: draft.account_id.clone(),
            mail_id: Some(draft.trigger_mail_id.clone()),
            draft_id: Some(draft.id.clone()),
            decision_type: decision_type::DRAFT_SENT.to_string(),
            impact: "reply".into(),
            action_description: "Human approved the AI draft; reply queued for SMTP delivery."
                .into(),
            result_description: "Draft marked sent and removed from the review queue.".into(),
            knowledge_refs: draft.knowledge_refs.clone(),
            knowledge_summary: None,
            ai_model: Some(draft.ai_model.clone()),
            input_tokens: None,
            output_tokens: None,
            latency_ms: Some(started.elapsed().as_millis() as i64),
        },
    )
    .await?;
    tx.commit().await.map_err(map_sqlx_err)?;

    // Queue semantics: the draft was consumed, so the review queue drops it.
    state.events.draft_discarded(id, Some("sent"));
    tracing::info!(
        event = "draft_approved",
        draft_id = %id,
        account_id = %draft.account_id,
        pending_id = %send.pending_id,
        "ai draft approved and queued for send"
    );

    Ok(ApproveDraftResult {
        sent_at,
        message_id: send.message_id,
        pending_id: Some(send.pending_id),
    })
}

/// Pending AI drafts for the E6 review queue (02 §Module E), newest first.
#[tauri::command]
pub async fn list_pending_drafts(
    state: State<'_, AppState>,
    params: ListAiDraftsParams,
) -> Result<Vec<AiDraft>, IpcError> {
    let limit = params
        .limit
        .unwrap_or(PENDING_DRAFTS_DEFAULT_LIMIT)
        .clamp(1, PENDING_DRAFTS_MAX_LIMIT);
    ai_draft_repo::list_pending(state.storage.db(), params.account_id.as_deref(), limit)
        .await
        .map_err(IpcError::from)
}

/// One AI draft by id. Named `get_ai_draft` (not dev/02's `get_draft`) to
/// avoid colliding with the T045 compose-draft command. Errors: `NOT_FOUND`.
#[tauri::command]
pub async fn get_ai_draft(state: State<'_, AppState>, id: String) -> Result<AiDraft, IpcError> {
    ai_draft_repo::get(state.storage.db(), &id)
        .await
        .map_err(IpcError::from)
}

/// Persist a user edit to the draft body (E6/T090 autosave). Sets
/// `is_edited`, moves the status to `edited`, never touches `body_original`.
/// Errors: `NOT_FOUND`, `FORBIDDEN` (draft already sent/discarded/expired).
#[tauri::command]
pub async fn update_draft_body(
    state: State<'_, AppState>,
    id: String,
    body_current: String,
) -> Result<AiDraft, IpcError> {
    do_update_draft_body(&state, &id, &body_current)
        .await
        .map_err(IpcError::from)
}

/// Approve a draft: send the reply via the queued SMTP path (10 s cancel
/// window), mark the draft `sent`, and write the `draft_sent` audit record
/// atomically. Errors: `NOT_FOUND`, `FORBIDDEN` (not pending/edited — e.g. a
/// double-approve), `SMTP_SEND_FAILED`, `VALIDATION`.
#[tauri::command]
pub async fn approve_draft(
    state: State<'_, AppState>,
    id: String,
) -> Result<ApproveDraftResult, IpcError> {
    do_approve_draft(&state, &id).await.map_err(IpcError::from)
}

/// Discard a draft from the review queue (`reason` defaults to `user`).
/// Errors: `NOT_FOUND`, `FORBIDDEN` (already sent).
#[tauri::command]
pub async fn discard_draft(
    state: State<'_, AppState>,
    id: String,
    reason: Option<String>,
) -> Result<(), IpcError> {
    do_discard_draft(&state, &id, reason.as_deref())
        .await
        .map_err(IpcError::from)
}

/// T090 edge-case backstop for the frontend-driven 5 s undo window: if the
/// `approve_draft` invoke has not run yet the draft is still `pending`/
/// `edited` and this is a no-op returning the row; once `sent`, cancellation
/// is `FORBIDDEN` (use `cancel_send(pendingId)` inside the SMTP window).
#[tauri::command]
pub async fn cancel_draft_send(
    state: State<'_, AppState>,
    id: String,
) -> Result<AiDraft, IpcError> {
    ai_draft_repo::cancel_send(state.storage.db(), &id)
        .await
        .map_err(IpcError::from)
}

// ── E7 audit log (T088, 02 §Module E) ────────────────────────────────────────

/// Filtered, paginated audit-log rows, newest first, with the trigger mail's
/// subject joined in for display (F_E7 §4.4/§4.5).
#[tauri::command]
pub async fn list_ai_decisions(
    state: State<'_, AppState>,
    params: ListDecisionsParams,
) -> Result<Vec<AiDecisionRow>, IpcError> {
    crate::ai::audit::repo::list_decisions(state.storage.db(), &params)
        .await
        .map_err(IpcError::from)
}

/// Aggregated audit statistics over `[sinceUnix, untilUnix]` — one SQL pass
/// (F_E7 §4.6). `accountId = null` spans all accounts.
#[tauri::command]
pub async fn get_ai_decisions_summary(
    state: State<'_, AppState>,
    account_id: Option<String>,
    since_unix: i64,
    until_unix: i64,
) -> Result<DecisionSummary, IpcError> {
    crate::ai::audit::repo::get_decisions_summary(
        state.storage.db(),
        account_id.as_deref(),
        since_unix,
        until_unix,
    )
    .await
    .map_err(IpcError::from)
}

/// Export the audit window to `{app data}/exports/ai_decisions_{ts}.{csv|json}`
/// and return the file path. Descriptions and subjects are excluded from the
/// file (F_E7 §4.7 privacy boundary). Errors: `FS_DISK_FULL`,
/// `FS_PERMISSION_DENIED`.
#[tauri::command]
pub async fn export_ai_decisions(
    state: State<'_, AppState>,
    params: ExportAiDecisionsParams,
) -> Result<String, IpcError> {
    crate::ai::audit::repo::export_decisions_to_file(&state, &params)
        .await
        .map_err(IpcError::from)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ai::matrix::{MatrixCell, MatrixEntry, ProviderAssignment};
    use crate::ai::mock::MockProvider;
    use crate::ai::{AccountAiConfig, AiProviderClient, Capability};
    use crate::types::ErrorCode;
    use crate::util::{new_uuid, now_unix};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Insert a minimal account + its `account_ai_settings` row (schema defaults).
    async fn seed_account(state: &AppState) -> String {
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
            "INSERT INTO account_ai_settings (account_id, auth_level, updated_at) VALUES (?, 1, ?)",
        )
        .bind(&id)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        id
    }

    #[tokio::test]
    async fn get_settings_unknown_account_is_not_found() {
        let (state, _rx) = AppState::test_state().await;
        let err = fetch_settings(&state, "missing").await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn get_settings_returns_schema_defaults() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state).await;
        let settings = fetch_settings(&state, &account).await.unwrap();
        assert_eq!(settings.account_id, account);
        assert_eq!(settings.ai_provider, AiProvider::None);
        assert_eq!(settings.ai_model, None);
        assert_eq!(settings.daily_query_limit, 10);
        assert!(settings.t4_enabled);
        assert!(!settings.t5_enabled);
    }

    #[tokio::test]
    async fn update_is_partial_and_bumps_updated_at() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state).await;
        let before = fetch_settings(&state, &account).await.unwrap();

        let params = UpdateAiSettingsParams {
            ai_provider: Some(AiProvider::Openai),
            ai_model: Some("gpt-4o".into()),
            daily_query_limit: Some(25),
            ..Default::default()
        };
        let updated = do_update(&state, &account, params).await.unwrap();

        assert_eq!(updated.ai_provider, AiProvider::Openai);
        assert_eq!(updated.ai_model.as_deref(), Some("gpt-4o"));
        assert_eq!(updated.daily_query_limit, 25);
        // Untouched fields keep their values.
        assert_eq!(updated.auth_level, before.auth_level);
        assert_eq!(updated.t1_enabled, before.t1_enabled);
        assert_eq!(updated.e3_min_history, before.e3_min_history);
        assert!(updated.updated_at >= before.updated_at);
    }

    #[tokio::test]
    async fn update_unknown_account_is_not_found() {
        let (state, _rx) = AppState::test_state().await;
        let err = do_update(&state, "missing", UpdateAiSettingsParams::default())
            .await
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn update_rejects_zero_daily_limit_and_empty_key() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state).await;

        let err = do_update(
            &state,
            &account,
            UpdateAiSettingsParams {
                daily_query_limit: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), ErrorCode::Validation);

        let err = do_update(
            &state,
            &account,
            UpdateAiSettingsParams {
                ai_api_key: Some("   ".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), ErrorCode::Validation);
    }

    // ── T087 — authorization-level enforcement ──────────────────────────────

    async fn account_auth_level(state: &AppState, account_id: &str) -> i64 {
        let (level,): (i64,) = sqlx::query_as("SELECT auth_level FROM accounts WHERE id = ?")
            .bind(account_id)
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        level
    }

    /// Insert one `draft_sent` audit row for the account (the E3 gate input).
    async fn seed_draft_sent_decision(state: &AppState, account_id: &str) {
        sqlx::query(
            "INSERT INTO ai_decisions (id, account_id, decision_type, impact, \
                 action_description, result_description, created_at) \
             VALUES (?, ?, 'draft_sent', 'reply', 'Approved draft sent.', 'Sent.', ?)",
        )
        .bind(new_uuid())
        .bind(account_id)
        .bind(now_unix())
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn auth_level_change_syncs_accounts_table() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state).await;
        assert_eq!(account_auth_level(&state, &account).await, 1);

        let updated = do_update(
            &state,
            &account,
            UpdateAiSettingsParams {
                auth_level: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.auth_level, 2);
        // Mirror column and authoritative column agree (same transaction).
        assert_eq!(account_auth_level(&state, &account).await, 2);
    }

    #[tokio::test]
    async fn auth_level_out_of_range_is_validation() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state).await;
        for level in [0u8, 4] {
            let err = do_update(
                &state,
                &account,
                UpdateAiSettingsParams {
                    auth_level: Some(level),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
            assert_eq!(err.code(), ErrorCode::Validation);
        }
    }

    #[tokio::test]
    async fn e3_gate_rejects_without_enough_sent_drafts() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state).await;
        // Default e3_min_history = 3; no draft_sent rows exist.
        let err = do_update(
            &state,
            &account,
            UpdateAiSettingsParams {
                auth_level: Some(3),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), ErrorCode::Validation);
        // Neither table moved.
        let settings = fetch_settings(&state, &account).await.unwrap();
        assert_eq!(settings.auth_level, 1);
        assert_eq!(account_auth_level(&state, &account).await, 1);
    }

    #[tokio::test]
    async fn e3_gate_passes_at_the_threshold() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state).await;
        for _ in 0..3 {
            seed_draft_sent_decision(&state, &account).await;
        }
        let updated = do_update(
            &state,
            &account,
            UpdateAiSettingsParams {
                auth_level: Some(3),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.auth_level, 3);
        assert_eq!(account_auth_level(&state, &account).await, 3);
    }

    #[tokio::test]
    async fn update_never_returns_key_material() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state).await;
        let updated = do_update(
            &state,
            &account,
            UpdateAiSettingsParams {
                ai_model: Some("gpt-4o".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        // Structural check: the DTO has no key field, so its serialization can
        // never carry one — and the Debug of the params type redacts inbound keys.
        let wire = serde_json::to_string(&updated).unwrap();
        assert!(!wire.contains("apiKey"));
        assert!(!wire.contains("ai_api_key_ref"));

        let params = UpdateAiSettingsParams {
            ai_api_key: Some("sk-very-secret".into()),
            ..Default::default()
        };
        let debugged = format!("{params:?}");
        assert!(!debugged.contains("sk-very-secret"));
        assert!(debugged.contains("***"));
    }

    #[tokio::test]
    async fn verify_local_providers_are_not_verifiable() {
        for provider in [AiProvider::LocalOnnx, AiProvider::None] {
            let result = do_verify(VerifyAiProviderParams {
                provider,
                model: "any-model".into(),
                api_key: None,
                base_url: None,
            })
            .await;
            assert!(!result.ok);
            assert!(result.error_message.is_some());
        }
    }

    #[tokio::test]
    async fn verify_requires_a_model_name() {
        let result = do_verify(VerifyAiProviderParams {
            provider: AiProvider::Openai,
            model: "  ".into(),
            api_key: Some("sk-test".into()),
            base_url: None,
        })
        .await;
        assert!(!result.ok);
        assert!(result.error_message.is_some());
    }

    #[tokio::test]
    async fn verify_openai_dispatches_to_probe_in_band() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "model": "gpt-4o-2024-08-06",
                "choices": [{
                    "message": { "role": "assistant", "content": "ok" },
                    "finish_reason": "stop"
                }]
            })))
            .mount(&server)
            .await;

        let result = do_verify(VerifyAiProviderParams {
            provider: AiProvider::Openai,
            model: "gpt-4o".into(),
            api_key: Some("sk-transient-test-key".into()),
            base_url: Some(server.uri()),
        })
        .await;
        assert!(result.ok);
        assert_eq!(result.model_name.as_deref(), Some("gpt-4o-2024-08-06"));
        assert_eq!(result.error_message, None);
    }

    #[tokio::test]
    async fn verify_failure_is_in_band_and_sanitized() {
        // Connection refused → Ok(result) with ok=false; no key, no body text.
        let result = do_verify(VerifyAiProviderParams {
            provider: AiProvider::Openai,
            model: "gpt-4o".into(),
            api_key: Some("sk-transient-test-key".into()),
            base_url: Some("http://127.0.0.1:1".into()),
        })
        .await;
        assert!(!result.ok);
        let message = result.error_message.unwrap();
        assert!(!message.contains("sk-"));
        assert!(!message.contains("Bearer"));
    }

    // ── F4 provider matrix (T065) ────────────────────────────────────────────

    /// Seed an account whose base AI provider/model are set (matrix tests
    /// need a non-`none` base to fall back to).
    async fn seed_ai_account(state: &AppState, provider: &str, model: Option<&str>) -> String {
        let id = seed_account(state).await;
        sqlx::query(
            "UPDATE account_ai_settings SET ai_provider = ?, ai_model = ? WHERE account_id = ?",
        )
        .bind(provider)
        .bind(model)
        .bind(&id)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        id
    }

    fn assignment(provider: AiProvider, model: &str) -> ProviderAssignment {
        ProviderAssignment {
            provider,
            model: model.into(),
            base_url: None,
        }
    }

    fn one_entry_matrix(cap: Capability, cell: MatrixCell) -> CapabilityMatrix {
        CapabilityMatrix {
            entries: vec![MatrixEntry {
                capability: cap,
                cell,
            }],
        }
    }

    async fn stored_matrix_column(state: &AppState, account_id: &str) -> Option<String> {
        let (raw,): (Option<String>,) =
            sqlx::query_as("SELECT provider_matrix FROM account_ai_settings WHERE account_id = ?")
                .bind(account_id)
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        raw
    }

    #[tokio::test]
    async fn get_matrix_null_column_returns_computed_defaults_without_persisting() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_ai_account(&state, "openai", Some("gpt-4o")).await;
        state
            .ai
            .register(Arc::new(MockProvider::healthy(AiProvider::LocalOnnx)));
        state
            .ai
            .register(Arc::new(MockProvider::healthy(AiProvider::Openai)));

        let matrix = do_get_matrix(&state, &account).await.unwrap();
        // With a local_onnx adapter registered, E4/E5 prefer local (F_F4 §4.1).
        assert_eq!(
            matrix
                .cell(Capability::RiskReason)
                .unwrap()
                .primary
                .provider,
            AiProvider::LocalOnnx
        );
        assert_eq!(
            matrix
                .cell(Capability::StyleProfile)
                .unwrap()
                .primary
                .provider,
            AiProvider::LocalOnnx
        );
        assert_eq!(
            matrix
                .cell(Capability::DraftReply)
                .unwrap()
                .primary
                .provider,
            AiProvider::Openai
        );
        assert_eq!(
            matrix.cell(Capability::Summarize).unwrap().primary.model,
            "gpt-4o"
        );
        // Defaults are computed on read, never written (T065 §3).
        assert_eq!(stored_matrix_column(&state, &account).await, None);
    }

    #[tokio::test]
    async fn get_matrix_unknown_account_is_not_found() {
        let (state, _rx) = AppState::test_state().await;
        let err = do_get_matrix(&state, "missing").await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn update_matrix_backup_chain_too_long_is_validation() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_ai_account(&state, "anthropic", None).await;

        let matrix = one_entry_matrix(
            Capability::DraftReply,
            MatrixCell {
                primary: assignment(AiProvider::Anthropic, "claude-sonnet-4-5"),
                backups: vec![
                    assignment(AiProvider::Openai, "gpt-4o"),
                    assignment(AiProvider::Ollama, "llama3.1-8b"),
                    assignment(AiProvider::LocalOnnx, ""),
                ],
            },
        );
        let err = do_update_matrix(&state, &account, &matrix)
            .await
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::Validation);
        // A rejected save writes nothing.
        assert_eq!(stored_matrix_column(&state, &account).await, None);
    }

    #[tokio::test]
    async fn update_matrix_primary_equals_backup_is_validation() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_ai_account(&state, "openai", None).await;

        let matrix = one_entry_matrix(
            Capability::Summarize,
            MatrixCell {
                primary: assignment(AiProvider::Openai, "gpt-4o"),
                backups: vec![assignment(AiProvider::Openai, "gpt-4o-mini")],
            },
        );
        let err = do_update_matrix(&state, &account, &matrix)
            .await
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::Validation);
        assert_eq!(stored_matrix_column(&state, &account).await, None);
    }

    #[tokio::test]
    async fn update_matrix_persists_and_returns_nonblocking_warnings() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_ai_account(&state, "anthropic", None).await;

        // Cloud primary on RiskReason draws an advisory warning (F_F4 §4.5)
        // but the save must land regardless.
        let matrix = one_entry_matrix(
            Capability::RiskReason,
            MatrixCell {
                primary: assignment(AiProvider::Anthropic, "claude-opus-4-1"),
                backups: vec![assignment(AiProvider::LocalOnnx, "")],
            },
        );
        let warnings = do_update_matrix(&state, &account, &matrix).await.unwrap();
        assert!(warnings.iter().any(|w| w.code == "high_cost_cloud"));

        let fetched = do_get_matrix(&state, &account).await.unwrap();
        assert_eq!(fetched, matrix);
    }

    #[tokio::test]
    async fn update_matrix_bumps_stamp_so_resolve_cache_invalidates() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_ai_account(&state, "openai", None).await;
        // Backdate the settings stamp so the matrix write provably moves it.
        sqlx::query("UPDATE account_ai_settings SET updated_at = 1000 WHERE account_id = ?")
            .bind(&account)
            .execute(state.storage.db().pool())
            .await
            .unwrap();

        let seen_models = Arc::new(std::sync::Mutex::new(Vec::<Option<String>>::new()));
        let seen = seen_models.clone();
        state.ai.register_factory(
            AiProvider::Openai,
            Arc::new(move |cfg: &AccountAiConfig| {
                seen.lock().unwrap().push(cfg.model.clone());
                Ok(Arc::new(MockProvider::healthy(AiProvider::Openai))
                    as Arc<dyn AiProviderClient>)
            }),
        );

        // Warm the cache: one build, the second resolve is served from it.
        state
            .ai
            .resolve(&account, Capability::DraftReply)
            .await
            .unwrap();
        state
            .ai
            .resolve(&account, Capability::DraftReply)
            .await
            .unwrap();
        assert_eq!(seen_models.lock().unwrap().len(), 1);

        let matrix = one_entry_matrix(
            Capability::DraftReply,
            MatrixCell {
                primary: assignment(AiProvider::Openai, "gpt-4o-mini"),
                backups: Vec::new(),
            },
        );
        do_update_matrix(&state, &account, &matrix).await.unwrap();

        // The matrix UPDATE must set updated_at — that stamp is what expires
        // the registry's cached adapter (T065 §7).
        let (updated_at,): (i64,) =
            sqlx::query_as("SELECT updated_at FROM account_ai_settings WHERE account_id = ?")
                .bind(&account)
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        assert!(updated_at > 1000);

        state
            .ai
            .resolve(&account, Capability::DraftReply)
            .await
            .unwrap();
        let models = seen_models.lock().unwrap().clone();
        assert_eq!(models, vec![None, Some("gpt-4o-mini".to_string())]);
    }

    #[tokio::test]
    async fn reset_matrix_writes_defaults_and_returns_them() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_ai_account(&state, "openai", Some("gpt-4o")).await;
        state
            .ai
            .register(Arc::new(MockProvider::healthy(AiProvider::LocalOnnx)));

        // Start from a customized matrix, then reset.
        let custom = one_entry_matrix(
            Capability::DraftReply,
            MatrixCell {
                primary: assignment(AiProvider::Anthropic, "claude-sonnet-4-5"),
                backups: Vec::new(),
            },
        );
        do_update_matrix(&state, &account, &custom).await.unwrap();

        let reset = do_reset_matrix(&state, &account).await.unwrap();
        assert_eq!(
            reset.cell(Capability::RiskReason).unwrap().primary.provider,
            AiProvider::LocalOnnx
        );
        assert_eq!(
            reset.cell(Capability::DraftReply).unwrap().primary.provider,
            AiProvider::Openai
        );
        // Reset persists: a fresh read returns the same defaults.
        let fetched = do_get_matrix(&state, &account).await.unwrap();
        assert_eq!(fetched, reset);
        assert!(stored_matrix_column(&state, &account).await.is_some());
    }

    #[tokio::test]
    async fn batch_update_merges_cells_into_the_effective_matrix() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_ai_account(&state, "openai", Some("gpt-4o")).await;

        let updates = vec![BatchMatrixUpdate {
            account_id: account.clone(),
            capability: Capability::DraftReply,
            cell: MatrixCell {
                primary: assignment(AiProvider::Anthropic, "claude-sonnet-4-5"),
                backups: vec![assignment(AiProvider::Openai, "gpt-4o")],
            },
        }];
        do_batch_update_matrix(&state, &updates).await.unwrap();

        let stored = do_get_matrix(&state, &account).await.unwrap();
        assert_eq!(
            stored
                .cell(Capability::DraftReply)
                .unwrap()
                .primary
                .provider,
            AiProvider::Anthropic
        );
        // The first batch edit materializes the computed defaults for the
        // untouched capabilities instead of dropping them.
        assert_eq!(
            stored.cell(Capability::Summarize).unwrap().primary.provider,
            AiProvider::Openai
        );
        assert!(stored_matrix_column(&state, &account).await.is_some());
    }

    #[tokio::test]
    async fn batch_update_rejects_the_whole_batch_on_one_invalid_cell() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_ai_account(&state, "openai", None).await;

        let updates = vec![
            BatchMatrixUpdate {
                account_id: account.clone(),
                capability: Capability::DraftReply,
                cell: MatrixCell {
                    primary: assignment(AiProvider::Anthropic, "claude-sonnet-4-5"),
                    backups: Vec::new(),
                },
            },
            // Invalid: backup repeats the primary provider.
            BatchMatrixUpdate {
                account_id: account.clone(),
                capability: Capability::Summarize,
                cell: MatrixCell {
                    primary: assignment(AiProvider::Openai, "gpt-4o"),
                    backups: vec![assignment(AiProvider::Openai, "gpt-4o-mini")],
                },
            },
        ];
        let err = do_batch_update_matrix(&state, &updates).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::Validation);
        // Up-front validation means the valid first item was not written either.
        assert_eq!(stored_matrix_column(&state, &account).await, None);
    }

    // ── F5 user-forced degradation (T067) ────────────────────────────────────

    use crate::ai::fallback::{FallbackRouter, InvokeOutcome, AI_DISABLE_UNTIL_KEY};
    use crate::ai::types::ChatRequest;
    use crate::storage::SettingRepo;

    #[tokio::test]
    async fn set_ai_disabled_writes_and_clears_the_setting() {
        let (state, _rx) = AppState::test_state().await;

        do_set_ai_disabled(&state, Some(1_900_000_000))
            .await
            .unwrap();
        let raw = SettingRepo::new(state.storage.db())
            .get(AI_DISABLE_UNTIL_KEY)
            .await
            .unwrap();
        assert_eq!(raw.as_deref(), Some("1900000000"));

        // `null` restores AI immediately — the key is gone.
        do_set_ai_disabled(&state, None).await.unwrap();
        let raw = SettingRepo::new(state.storage.db())
            .get(AI_DISABLE_UNTIL_KEY)
            .await
            .unwrap();
        assert!(raw.is_none());
    }

    #[tokio::test]
    async fn set_ai_disabled_rejects_negative_timestamp() {
        let (state, _rx) = AppState::test_state().await;
        let err = do_set_ai_disabled(&state, Some(-1)).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::Validation);
    }

    #[tokio::test]
    async fn set_ai_disabled_stops_router_resolution() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_ai_account(&state, "openai", None).await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        state.ai.register(mock.clone());
        let router = FallbackRouter::new(
            state.ai.clone(),
            state.storage.db().clone(),
            state.events.clone(),
        );

        do_set_ai_disabled(&state, Some(now_unix() + 86_400))
            .await
            .unwrap();
        let outcome = router
            .invoke(
                &account,
                Capability::DraftReply,
                ChatRequest::simple(
                    "mock-model",
                    "summarize the latest thread",
                    Capability::DraftReply,
                ),
                &[],
            )
            .await
            .unwrap();
        let InvokeOutcome::Degraded(decision) = outcome else {
            panic!("expected user_disabled degrade");
        };
        assert_eq!(decision.reason, "user_disabled");
        // No provider was touched while AI is user-disabled.
        assert_eq!(mock.chat_call_count(), 0);

        // Restoring (`null`) lets the next call resolve and complete.
        do_set_ai_disabled(&state, None).await.unwrap();
        let outcome = router
            .invoke(
                &account,
                Capability::DraftReply,
                ChatRequest::simple(
                    "mock-model",
                    "summarize the latest thread",
                    Capability::DraftReply,
                ),
                &[],
            )
            .await
            .unwrap();
        assert!(matches!(outcome, InvokeOutcome::Completed { .. }));
        assert_eq!(mock.chat_call_count(), 1);
    }

    // ── Provider config UI (T068) ────────────────────────────────────────────

    /// A localhost base URL where nothing listens (bind, read port, drop).
    fn refused_base_url() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        format!("http://127.0.0.1:{port}")
    }

    #[test]
    fn scan_results_map_to_ollama_endpoints_in_order() {
        let endpoints = to_local_endpoints(vec![
            "http://localhost:11434".into(),
            "http://127.0.0.1:11434".into(),
        ]);
        assert_eq!(endpoints.len(), 2);
        assert!(endpoints.iter().all(|e| e.provider == AiProvider::Ollama));
        assert_eq!(endpoints[0].base_url, "http://localhost:11434");
        // camelCase wire shape for the frontend DTO mirror.
        let wire = serde_json::to_value(&endpoints[0]).unwrap();
        assert_eq!(wire["baseUrl"], "http://localhost:11434");
        assert_eq!(wire["provider"], "ollama");

        assert!(to_local_endpoints(Vec::new()).is_empty());
    }

    #[tokio::test]
    async fn list_ollama_models_projects_daemon_tags() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "models": [
                    {
                        "name": "llama3:8b",
                        "size": 4_661_224_676u64,
                        "details": { "parameter_size": "8B", "quantization_level": "Q4_0" }
                    },
                    { "name": "qwen2.5:14b" }
                ]
            })))
            .mount(&server)
            .await;

        let uri = server.uri();
        let models = do_list_ollama_models(Some(&uri)).await.unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].name, "llama3:8b");
        assert_eq!(models[0].size_bytes, 4_661_224_676);
        assert_eq!(models[0].parameter_size.as_deref(), Some("8B"));
        assert_eq!(models[0].quantization.as_deref(), Some("Q4_0"));
        // Daemon omitted size/details → defaults, not an error.
        assert_eq!(models[1].size_bytes, 0);
        assert_eq!(models[1].parameter_size, None);
        // camelCase wire shape.
        let wire = serde_json::to_value(&models[0]).unwrap();
        assert_eq!(wire["sizeBytes"], 4_661_224_676u64);
        assert_eq!(wire["parameterSize"], "8B");
    }

    #[tokio::test]
    async fn list_ollama_models_unreachable_is_ai_provider_unreachable() {
        let base = refused_base_url();
        let err = do_list_ollama_models(Some(&base)).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::AiProviderUnreachable);
    }

    #[tokio::test]
    async fn configured_providers_skip_none_and_flag_local_and_availability() {
        let (state, _rx) = AppState::test_state().await;
        // Three accounts: provider `none` (filtered out), openai (cloud, no
        // adapter registered), ollama (local, registered).
        let _unconfigured = seed_account(&state).await;
        let cloud = seed_ai_account(&state, "openai", Some("gpt-4o")).await;
        let local = seed_ai_account(&state, "ollama", Some("llama3:8b")).await;
        state
            .ai
            .register(Arc::new(MockProvider::healthy(AiProvider::Ollama)));

        let list = do_list_configured_providers(&state).await.unwrap();
        assert_eq!(list.len(), 2);

        let cloud_row = list.iter().find(|p| p.account_id == cloud).unwrap();
        assert_eq!(cloud_row.provider, AiProvider::Openai);
        assert_eq!(cloud_row.model.as_deref(), Some("gpt-4o"));
        assert!(!cloud_row.is_local);
        // registry.registered() has no openai adapter in this test build.
        assert!(!cloud_row.available);

        let local_row = list.iter().find(|p| p.account_id == local).unwrap();
        assert_eq!(local_row.provider, AiProvider::Ollama);
        assert!(local_row.is_local);
        assert!(local_row.available);
        assert_eq!(local_row.email, format!("{local}@example.com"));
        assert_eq!(local_row.display_name, "Work");
        assert_eq!(local_row.color_token, "slate");
        assert_eq!(local_row.auth_level, 1);
    }

    #[tokio::test]
    #[ignore = "requires interactive macOS Keychain access (writes a real AiApiKey item)"]
    async fn configured_providers_never_carry_key_material() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_ai_account(&state, "openai", Some("gpt-4o")).await;
        do_update(
            &state,
            &account,
            UpdateAiSettingsParams {
                ai_api_key: Some("sk-super-secret".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let list = do_list_configured_providers(&state).await.unwrap();
        assert_eq!(list.len(), 1);
        // Structural check: the row type has no key field, so the wire JSON
        // can never carry key material or the Keychain reference (ADR-0004).
        let wire = serde_json::to_string(&list).unwrap();
        assert!(!wire.contains("sk-super-secret"));
        assert!(!wire.contains("apiKey"));
        assert!(!wire.contains("ai_api_key_ref"));
    }

    // ── E6 queue + approve/cancel send (T080/T090) ───────────────────────────

    /// Insert a received trigger mail with threading headers.
    async fn seed_trigger_mail(state: &AppState, id: &str, account_id: &str) {
        let now = now_unix();
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, \"references\", subject, \
                 from_email, to_addrs, date_sent, date_received, created_at, updated_at) \
             VALUES (?, ?, ?, '<root@x>', 'Renewal terms', 'daniel@vendorco.example', '[]', \
                 ?, ?, 0, 0)",
        )
        .bind(id)
        .bind(account_id)
        .bind(format!("<{id}@x>"))
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    /// Insert an `ai_drafts` row directly (the repo INSERT is engine-only).
    async fn seed_ai_draft(state: &AppState, account_id: &str, status: &str) -> String {
        let id = new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO ai_drafts (id, trigger_mail_id, account_id, to_addr, cc_addrs, \
                 subject, body_original, body_current, trigger_mode, ai_model, \
                 knowledge_refs, status, created_at, updated_at) \
             VALUES (?, 'm1', ?, '{\"name\":\"Daniel\",\"email\":\"daniel@vendorco.example\"}', \
                 '[]', 'Re: Renewal terms', 'Original body.', 'Original body.', 'E2_semi', \
                 'gpt-4o', '[\"k1\"]', ?, ?, ?)",
        )
        .bind(&id)
        .bind(account_id)
        .bind(status)
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        id
    }

    /// State + active account (`smtp` fields set so schedule_send works) +
    /// trigger mail `m1`.
    async fn draft_test_state() -> (AppState, String) {
        let (state, _rx) = AppState::test_state().await;
        let account = new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, smtp_host, smtp_port, \
                 color_token, badge_label, is_active, created_at, updated_at) \
             VALUES (?, ?, 'Work', 'imap', 'smtp.example.com', 587, 'slate', 'W', 1, ?, ?)",
        )
        .bind(&account)
        .bind(format!("{account}@example.com"))
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        seed_trigger_mail(&state, "m1", &account).await;
        (state, account)
    }

    async fn decision_rows(state: &AppState, decision_type: &str) -> i64 {
        let (n,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM ai_decisions WHERE decision_type = ?")
                .bind(decision_type)
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        n
    }

    #[tokio::test]
    async fn update_draft_body_edits_and_audits() {
        let (state, account) = draft_test_state().await;
        let id = seed_ai_draft(&state, &account, "pending").await;

        let updated = do_update_draft_body(&state, &id, "Edited reply.")
            .await
            .unwrap();
        assert_eq!(updated.body_current, "Edited reply.");
        assert_eq!(updated.body_original, "Original body.");
        assert!(updated.is_edited);
        assert_eq!(updated.status, "edited");
        assert_eq!(decision_rows(&state, "draft_edited").await, 1);
    }

    #[tokio::test]
    async fn approve_draft_happy_path_marks_sent_and_audits_atomically() {
        let (state, account) = draft_test_state().await;
        let id = seed_ai_draft(&state, &account, "pending").await;

        let result = do_approve_draft(&state, &id).await.unwrap();
        assert!(result.message_id.contains("seekermail.local"));
        assert!(result.pending_id.is_some());
        assert!(result.sent_at > 0);

        let draft = crate::ai::draft::repo::get(state.storage.db(), &id)
            .await
            .unwrap();
        assert_eq!(draft.status, "sent");
        assert_eq!(draft.sent_at, Some(result.sent_at));
        assert_eq!(decision_rows(&state, "draft_sent").await, 1);
        // The audit row carries identifiers, not the body.
        let (action,): (String,) = sqlx::query_as(
            "SELECT action_description FROM ai_decisions WHERE decision_type = 'draft_sent'",
        )
        .fetch_one(state.storage.db().pool())
        .await
        .unwrap();
        assert!(!action.contains("Original body."));
    }

    #[tokio::test]
    async fn approve_draft_twice_is_forbidden_without_second_send() {
        let (state, account) = draft_test_state().await;
        let id = seed_ai_draft(&state, &account, "edited").await;

        do_approve_draft(&state, &id).await.unwrap();
        let err = do_approve_draft(&state, &id).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::Forbidden);
        // Exactly one audit row — the second call never reached the send path.
        assert_eq!(decision_rows(&state, "draft_sent").await, 1);
    }

    #[tokio::test]
    async fn approve_draft_on_inactive_account_leaves_status_unchanged() {
        let (state, account) = draft_test_state().await;
        let id = seed_ai_draft(&state, &account, "pending").await;
        sqlx::query("UPDATE accounts SET is_active = 0 WHERE id = ?")
            .bind(&account)
            .execute(state.storage.db().pool())
            .await
            .unwrap();

        let err = do_approve_draft(&state, &id).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::Forbidden);
        let draft = crate::ai::draft::repo::get(state.storage.db(), &id)
            .await
            .unwrap();
        assert_eq!(draft.status, "pending", "a failed send must not mark sent");
        assert_eq!(decision_rows(&state, "draft_sent").await, 0);
    }

    #[tokio::test]
    async fn discard_draft_defaults_reason_and_audits() {
        let (state, account) = draft_test_state().await;
        let id = seed_ai_draft(&state, &account, "pending").await;

        do_discard_draft(&state, &id, None).await.unwrap();
        let draft = crate::ai::draft::repo::get(state.storage.db(), &id)
            .await
            .unwrap();
        assert_eq!(draft.status, "discarded");
        assert_eq!(draft.discard_reason.as_deref(), Some("user"));
        assert_eq!(decision_rows(&state, "draft_discarded").await, 1);

        // A sent draft cannot be discarded.
        let sent = seed_ai_draft(&state, &account, "sent").await;
        let err = do_discard_draft(&state, &sent, None).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::Forbidden);
    }
}
