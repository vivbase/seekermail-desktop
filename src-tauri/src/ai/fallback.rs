//! F5 offline fallback router (T067, F_F5 §3–§6, dev/06 §7).
//!
//! [`FallbackRouter`] wraps [`AiRegistry`] — it never replaces it. Every AI
//! call from the D/E modules goes through [`FallbackRouter::invoke`], which:
//!
//! 1. resolves the primary provider and walks the backup chain
//!    ([`AiRegistry::resolve_backup`]) when the primary fails;
//! 2. classifies each [`ProviderError`] (retry in place / switch backup /
//!    needs human, F_F5 §4.1);
//! 3. puts failing providers into a cooldown (`Auth` 60 min, `RateLimited`
//!    per `Retry-After` else 10 min, transient 5 min, F_F5 §4.3);
//! 4. returns a [`DowngradeDecision`] when the whole chain is exhausted —
//!    **degradation always moves toward more human control** (dev/06 §7):
//!    an E3 full-auto account is downgraded to E2 (queue for human review),
//!    never silently skipped, never sent blind;
//! 5. writes one `ai_decisions` audit row per call, success or degraded,
//!    without ever storing prompt or completion text (dev/06 §9, 09 §5).
//!
//! Time-dependent policy (cooldown expiry, probe lead, user-disable window)
//! is factored into pure functions taking `now` so the matrix is unit-testable
//! with an injected [`Clock`]. The hold queue is in-memory for v0.7 (F_F5
//! §4.4: it intentionally does not survive a restart).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::{AppError, AppResult};
use crate::events::Emitter;
use crate::storage::{map_sqlx_err, Db, SettingRepo};
use crate::types::AiProvider;
use crate::util::{new_uuid, now_unix};

use super::provider::{health_with_retry, AiProviderClient, ProviderError};
use super::registry::AiRegistry;
use super::types::{Capability, ChatRequest, ChatResponse};

// ── Policy constants (F_F5 §4.3–§4.4, card §3) ──────────────────────────────

/// `Auth` cooldown: the key is wrong; only the user can fix it (60 min).
pub const COOLDOWN_AUTH_SECS: i64 = 3_600;
/// `RateLimited` cooldown when the provider sent no `Retry-After` (10 min).
pub const COOLDOWN_RATE_LIMITED_DEFAULT_SECS: i64 = 600;
/// Transient cooldown — unreachable / 5xx-class / unparseable body (5 min).
pub const COOLDOWN_TRANSIENT_SECS: i64 = 300;
/// A cooled provider is health-probed once it is this close to expiry.
pub const PROBE_LEAD_SECS: i64 = 60;
/// In-place retries for transient failures before switching backup (card §3).
const TRANSIENT_RETRY_MAX: u32 = 2;
/// Exponential backoff base for in-place retries (doubled per attempt).
#[cfg(not(test))]
const RETRY_BACKOFF_BASE_MS: u64 = 250;
/// Random jitter ceiling added to each retry backoff.
#[cfg(not(test))]
const RETRY_JITTER_MAX_MS: u64 = 250;
// Test builds compress the retry/catch-up pacing to low single-digit
// milliseconds so the suite runs on the real tokio clock. Policy is
// untouched — retry counts, backoff shape, and the bounded batch size are
// identical; only the wait lengths shrink. (A paused tokio clock cannot be
// used here: its auto-advance jumps virtual time into the sqlx pool's
// acquire deadline while a connection hand-back is mid-flight on the
// SQLite worker thread, failing healthy queries with PoolTimedOut.)
#[cfg(test)]
const RETRY_BACKOFF_BASE_MS: u64 = 2;
#[cfg(test)]
const RETRY_JITTER_MAX_MS: u64 = 2;
/// Bounded catch-up: at most this many held items are replayed per recovery
/// pass, so a backlog never stampedes a provider that just came back
/// (dev/06 §7).
pub const CATCH_UP_MAX_PER_RUN: usize = 10;
/// Throttle between two catch-up replays.
#[cfg(not(test))]
const CATCH_UP_DELAY_MS: u64 = 500;
#[cfg(test)]
const CATCH_UP_DELAY_MS: u64 = 2;
/// `app_settings` key holding the user-forced degradation deadline
/// (unix seconds; absent or JSON `null` = AI enabled, F_F5 §4.5).
pub const AI_DISABLE_UNTIL_KEY: &str = "ai.disable_until";

/// Placeholder prefix written to `ai_decisions.ai_model` for degraded calls.
const DOWNGRADED_MODEL_PREFIX: &str = "downgraded:";

// ── Wire types (specta-exported; consumed by T069 / module E events) ────────

/// What the caller should do with the task that could not be served
/// (F_F5 §4.2). Every option keeps or increases human control — there is no
/// "send anyway" action by design (dev/06 §7 golden rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SuggestedAction {
    /// Keep the task queued; replay when a provider recovers (E2/E3, E5).
    Hold,
    /// Drop this background task; the per-account offline policy decides how
    /// the mail is treated (E4 sensitivity pre-scan).
    Skip,
    /// Tell the human; the action was user-triggered (E1/D1/D2) or needs a
    /// configuration fix.
    PromptUser,
}

impl SuggestedAction {
    /// Stable tag for logs and the `ai_model` placeholder (identifiers only).
    pub fn as_str(self) -> &'static str {
        match self {
            SuggestedAction::Hold => "hold",
            SuggestedAction::Skip => "skip",
            SuggestedAction::PromptUser => "prompt_user",
        }
    }
}

/// The all-providers-failed outcome of [`FallbackRouter::invoke`]
/// (F_F5 §4.2). Carried by module-E events (`draft:degraded` etc.), so it is
/// specta-exported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DowngradeDecision {
    pub account_id: String,
    pub capability: Capability,
    /// `all_providers_unavailable` | `context_too_long` | `user_disabled` |
    /// `not_configured`.
    pub reason: String,
    /// `"provider (failure)"` entries in the order they were tried —
    /// identifiers and error classes only, never content (09 §5).
    pub failed_chain: Vec<String>,
    pub suggested_action: SuggestedAction,
    /// Unix seconds of the earliest cooldown expiry, when one exists.
    pub retry_after: Option<i64>,
    /// `true` when a full-auto (E3) account must fall back to semi-auto (E2):
    /// the pipeline marks the draft `pending` for human review instead of
    /// sending (dev/06 §7 — never skip, never send blind).
    pub should_downgrade_e3_to_e2: bool,
}

/// `ai:offline` payload — every configured provider is cooled or unreachable.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiOfflinePayload {
    pub reason: String,
}

/// `ai:online` payload — a cooled provider passed its recovery probe.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiOnlinePayload {
    pub recovered_provider: String,
}

// ── Internal state ───────────────────────────────────────────────────────────

/// Why a provider is cooling (drives the reset duration after a failed probe).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CooldownReason {
    /// 401/403 — needs user action; longest cooldown.
    Auth,
    /// 429 — waiting out the quota window.
    RateLimited,
    /// Network / 5xx-class / unparseable body.
    Transient,
}

impl CooldownReason {
    /// Default cooldown for this failure class — also the reset applied when
    /// a recovery probe fails (F_F5 §4.3).
    pub fn cooldown_secs(self) -> i64 {
        match self {
            CooldownReason::Auth => COOLDOWN_AUTH_SECS,
            CooldownReason::RateLimited => COOLDOWN_RATE_LIMITED_DEFAULT_SECS,
            CooldownReason::Transient => COOLDOWN_TRANSIENT_SECS,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CooldownReason::Auth => "auth",
            CooldownReason::RateLimited => "rate_limited",
            CooldownReason::Transient => "transient",
        }
    }
}

/// One cooling provider. Holds the client handle so the recovery probe can
/// run `health()` without re-resolving through the registry.
#[derive(Clone)]
pub struct CooldownEntry {
    /// Unix seconds when the cooldown lapses on its own.
    pub until: i64,
    pub reason: CooldownReason,
    client: Arc<dyn AiProviderClient>,
}

impl CooldownEntry {
    /// Still cooling at `now`? Pure, so expiry logic is clock-testable.
    pub fn is_cooling(&self, now: i64) -> bool {
        self.until > now
    }

    /// Within the pre-expiry window where a background probe should run?
    pub fn probe_due(&self, now: i64) -> bool {
        self.is_cooling(now) && self.until - now <= PROBE_LEAD_SECS
    }
}

/// One task waiting for provider recovery (E2 hold, F_F5 §4.2). In-memory
/// only for v0.7 — a restart clears the queue (F_F5 §4.4).
#[derive(Clone)]
pub struct HoldEntry {
    pub account_id: String,
    pub capability: Capability,
    pub request: ChatRequest,
    /// Unix seconds when the task entered the queue.
    pub enqueued_at: i64,
}

struct RouterState {
    cooldowns: HashMap<AiProvider, CooldownEntry>,
    global_offline: bool,
    hold: Vec<HoldEntry>,
}

/// Read-only status snapshot for the T069 status indicator panel (F_F5 §5).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiRouterStatus {
    pub global_offline: bool,
    pub hold_count: u32,
    pub cooldowns: Vec<CooldownInfo>,
}

/// One cooling provider in the status snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CooldownInfo {
    pub provider: AiProvider,
    /// Unix seconds when the cooldown lapses.
    pub until: i64,
    /// `auth` | `rate_limited` | `transient`.
    pub reason: String,
}

// ── Injected clock ───────────────────────────────────────────────────────────

/// Unix-seconds clock, injectable so cooldown/probe timing is testable
/// without sleeping (card §8).
#[derive(Clone)]
pub struct Clock(Arc<dyn Fn() -> i64 + Send + Sync>);

impl Clock {
    /// Wall-clock seconds (production).
    pub fn system() -> Self {
        Clock(Arc::new(now_unix))
    }

    pub fn now(&self) -> i64 {
        (self.0)()
    }

    /// A manually advanced clock for tests: returns the clock and the shared
    /// cell that sets its current value.
    #[cfg(test)]
    pub fn manual(start: i64) -> (Self, Arc<std::sync::atomic::AtomicI64>) {
        let cell = Arc::new(std::sync::atomic::AtomicI64::new(start));
        let reader = cell.clone();
        (
            Clock(Arc::new(move || {
                reader.load(std::sync::atomic::Ordering::SeqCst)
            })),
            cell,
        )
    }
}

// ── Pure decision functions (clock-free, fully unit-testable) ───────────────

/// How [`FallbackRouter::invoke`] reacts to one provider failure (F_F5 §4.1,
/// card §3 step 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    /// Retry in place (bounded), then switch backup.
    RetryThenSwitch,
    /// Switch to the next backup immediately — retrying cannot help.
    SwitchImmediately,
    /// The problem is the prompt, not the provider: do not switch, hand the
    /// task back to a human.
    PromptUser,
    /// Locally canceled by the caller — abort the whole invocation.
    Aborted,
}

/// Classify one [`ProviderError`] (card §3 step 2).
pub fn classify(err: &ProviderError) -> FailureClass {
    match err {
        ProviderError::Unreachable(_) | ProviderError::BadResponse(_) => {
            FailureClass::RetryThenSwitch
        }
        ProviderError::RateLimited { retry_after: None } => FailureClass::RetryThenSwitch,
        ProviderError::RateLimited {
            retry_after: Some(_),
        }
        | ProviderError::Auth
        | ProviderError::ContentFiltered => FailureClass::SwitchImmediately,
        ProviderError::ContextTooLong => FailureClass::PromptUser,
        ProviderError::Canceled => FailureClass::Aborted,
    }
}

/// The cooldown a failure earns, if any (F_F5 §4.3). Pure in `now` so expiry
/// math is testable. `ContentFiltered` and `ContextTooLong` earn none — they
/// are content-specific, not provider-health signals.
pub fn cooldown_after(err: &ProviderError, now: i64) -> Option<(i64, CooldownReason)> {
    match err {
        ProviderError::Auth => Some((now + COOLDOWN_AUTH_SECS, CooldownReason::Auth)),
        ProviderError::RateLimited { retry_after } => {
            let secs = retry_after
                .map(|d| d.as_secs() as i64)
                .unwrap_or(COOLDOWN_RATE_LIMITED_DEFAULT_SECS);
            Some((now + secs, CooldownReason::RateLimited))
        }
        ProviderError::Unreachable(_) | ProviderError::BadResponse(_) => {
            Some((now + COOLDOWN_TRANSIENT_SECS, CooldownReason::Transient))
        }
        ProviderError::ContextTooLong
        | ProviderError::ContentFiltered
        | ProviderError::Canceled => None,
    }
}

/// `suggested_action` by capability + authorization level (card §3,
/// F_F5 §4.2). Every branch keeps or raises human control:
///
/// * `DraftReply` under E2/E3 → `Hold` (the mail waits; a draft is generated
///   on recovery, then a human reviews it);
/// * `DraftReply` under E1 (user-triggered) → `PromptUser`;
/// * `RiskReason` (E4 pre-scan) → `Skip` — the per-account offline-sensitivity
///   setting decides how the unscanned mail is treated;
/// * `Summarize` (user-triggered) → `PromptUser`;
/// * `StyleProfile` (E5) → `Hold` (shows as "pending" in settings).
pub fn suggested_action_for(cap: Capability, auth_level: u8) -> SuggestedAction {
    match cap {
        Capability::DraftReply => {
            if auth_level >= 2 {
                SuggestedAction::Hold
            } else {
                SuggestedAction::PromptUser
            }
        }
        Capability::RiskReason => SuggestedAction::Skip,
        Capability::Summarize => SuggestedAction::PromptUser,
        Capability::StyleProfile => SuggestedAction::Hold,
    }
}

/// E3 → E2 downgrade flag (card §6): a full-auto account whose task is held
/// must come back as a human-reviewed draft, never an automatic send.
pub fn should_downgrade_e3(auth_level: u8, action: SuggestedAction) -> bool {
    auth_level >= 3 && action == SuggestedAction::Hold
}

/// One `failed_chain` line: provider id + error class, never content (09 §5).
fn chain_entry(provider: AiProvider, failure: &str) -> String {
    format!("{} ({failure})", provider.as_str())
}

/// Short class tag for a `failed_chain` line (no payload text ever).
fn failure_tag(err: &ProviderError) -> &'static str {
    match err {
        ProviderError::Unreachable(_) => "unreachable",
        ProviderError::Auth => "auth",
        ProviderError::RateLimited { .. } => "rate_limited",
        ProviderError::ContextTooLong => "context_too_long",
        ProviderError::BadResponse(_) => "bad_response",
        ProviderError::ContentFiltered => "content_filtered",
        ProviderError::Canceled => "canceled",
    }
}

/// `ai_decisions.impact` bucket for a capability (01 schema: risk | reply |
/// identity | rule | context).
fn impact_for(cap: Capability) -> &'static str {
    match cap {
        Capability::DraftReply => "reply",
        Capability::RiskReason => "risk",
        Capability::Summarize | Capability::StyleProfile => "context",
    }
}

// ── Invocation outcome ───────────────────────────────────────────────────────

/// What one [`FallbackRouter::invoke`] produced.
#[derive(Debug)]
pub enum InvokeOutcome {
    /// A provider answered. `failed_chain` is non-empty when backups were
    /// walked before success.
    Completed {
        response: ChatResponse,
        provider: AiProvider,
        failed_chain: Vec<String>,
    },
    /// Nothing usable answered (or AI is disabled) — the caller must follow
    /// the decision's `suggested_action`.
    Degraded(DowngradeDecision),
}

/// Outcome of trying one provider (with in-place retries) inside the chain.
enum AttemptOutcome {
    Success(ChatResponse),
    Failed(ProviderError),
}

// ── FallbackRouter ───────────────────────────────────────────────────────────

/// The F5 degradation layer. Cheap to clone; all mutable state is shared
/// behind one `Arc<Mutex<…>>` (never held across an `await`).
#[derive(Clone)]
pub struct FallbackRouter {
    registry: AiRegistry,
    db: Db,
    events: Emitter,
    clock: Clock,
    inner: Arc<Mutex<RouterState>>,
}

impl FallbackRouter {
    pub fn new(registry: AiRegistry, db: Db, events: Emitter) -> Self {
        Self::with_clock(registry, db, events, Clock::system())
    }

    /// Construct with an explicit clock (tests inject a manual one).
    pub fn with_clock(registry: AiRegistry, db: Db, events: Emitter, clock: Clock) -> Self {
        Self {
            registry,
            db,
            events,
            clock,
            inner: Arc::new(Mutex::new(RouterState {
                cooldowns: HashMap::new(),
                global_offline: false,
                hold: Vec::new(),
            })),
        }
    }

    /// The unified F5 entry point (F_F5 §3). All D/E AI call sites route
    /// through here instead of `AiRegistry::resolve().chat()`.
    ///
    /// `knowledge_refs` is the caller-supplied GTE citation list for the
    /// `ai_decisions` audit row (dev/06 §9); pass `&[]` when none apply.
    ///
    /// Hard errors (unknown account, daily query limit, DB failures, local
    /// cancellation) surface as `Err`; provider unavailability never does —
    /// it becomes a [`DowngradeDecision`].
    pub async fn invoke(
        &self,
        account_id: &str,
        cap: Capability,
        request: ChatRequest,
        knowledge_refs: &[String],
    ) -> AppResult<InvokeOutcome> {
        let now = self.clock.now();
        let started = std::time::Instant::now();

        // 1. User-forced degradation (F_F5 §4.5): checked before anything
        //    touches a provider or the registry.
        if let Some(until) = self.disabled_until().await? {
            if now < until {
                let auth_level = self.auth_level(account_id).await?;
                let action = suggested_action_for(cap, auth_level);
                let decision = DowngradeDecision {
                    account_id: account_id.to_string(),
                    capability: cap,
                    reason: "user_disabled".into(),
                    failed_chain: Vec::new(),
                    suggested_action: action,
                    retry_after: Some(until),
                    should_downgrade_e3_to_e2: should_downgrade_e3(auth_level, action),
                };
                return self
                    .finish_degraded(decision, knowledge_refs, started)
                    .await;
            }
        }

        // 2. Lazy cooldown expiry + global-offline short-circuit (card §3:
        //    while globally offline, no network is attempted per call).
        let registered = self.registry.registered();
        let globally_offline = {
            let mut st = self.inner.lock().expect("fallback state lock poisoned");
            st.cooldowns.retain(|_, e| e.is_cooling(now));
            if st.global_offline && !all_cooling(&registered, &st.cooldowns, now) {
                // A cooldown lapsed on its own — leave the offline state and
                // let this call try the chain again.
                st.global_offline = false;
            }
            st.global_offline
        };
        if globally_offline {
            let auth_level = self.auth_level(account_id).await?;
            let action = suggested_action_for(cap, auth_level);
            let decision = DowngradeDecision {
                account_id: account_id.to_string(),
                capability: cap,
                reason: "all_providers_unavailable".into(),
                failed_chain: Vec::new(),
                suggested_action: action,
                retry_after: self.earliest_cooldown_expiry(now),
                should_downgrade_e3_to_e2: should_downgrade_e3(auth_level, action),
            };
            let hold = action == SuggestedAction::Hold;
            return self
                .finish_degraded_with_request(
                    decision,
                    knowledge_refs,
                    started,
                    hold.then(|| (account_id.to_string(), cap, request)),
                )
                .await;
        }

        let auth_level = self.auth_level(account_id).await?;

        // 3. Primary, then the backup chain.
        let mut failed_chain: Vec<String> = Vec::new();
        let mut exclude: Vec<AiProvider> = Vec::new();

        let primary = match self.registry.resolve(account_id, cap).await {
            Ok(client) => Some(client),
            Err(AppError::Forbidden(_)) => {
                // AI not configured / provider missing in this build — needs
                // a human, not a backup (F_F5 §4.1 "needs-human").
                let decision = DowngradeDecision {
                    account_id: account_id.to_string(),
                    capability: cap,
                    reason: "not_configured".into(),
                    failed_chain: Vec::new(),
                    suggested_action: SuggestedAction::PromptUser,
                    retry_after: None,
                    should_downgrade_e3_to_e2: false,
                };
                return self
                    .finish_degraded(decision, knowledge_refs, started)
                    .await;
            }
            Err(e) => return Err(e),
        };

        if let Some(client) = primary {
            match self
                .try_provider(client, &request, &mut failed_chain, &mut exclude, now)
                .await?
            {
                Some(ChainStep::Done(response, provider)) => {
                    return self
                        .finish_completed(
                            account_id,
                            cap,
                            response,
                            provider,
                            failed_chain,
                            knowledge_refs,
                            started,
                        )
                        .await;
                }
                Some(ChainStep::HumanNeeded(reason)) => {
                    let decision = DowngradeDecision {
                        account_id: account_id.to_string(),
                        capability: cap,
                        reason,
                        failed_chain,
                        suggested_action: SuggestedAction::PromptUser,
                        retry_after: None,
                        should_downgrade_e3_to_e2: false,
                    };
                    return self
                        .finish_degraded(decision, knowledge_refs, started)
                        .await;
                }
                None => {} // provider failed → walk the backups
            }
        }

        loop {
            let backup = self
                .registry
                .resolve_backup(account_id, cap, &exclude)
                .await?;
            let Some(client) = backup else { break };
            match self
                .try_provider(client, &request, &mut failed_chain, &mut exclude, now)
                .await?
            {
                Some(ChainStep::Done(response, provider)) => {
                    return self
                        .finish_completed(
                            account_id,
                            cap,
                            response,
                            provider,
                            failed_chain,
                            knowledge_refs,
                            started,
                        )
                        .await;
                }
                Some(ChainStep::HumanNeeded(reason)) => {
                    let decision = DowngradeDecision {
                        account_id: account_id.to_string(),
                        capability: cap,
                        reason,
                        failed_chain,
                        suggested_action: SuggestedAction::PromptUser,
                        retry_after: None,
                        should_downgrade_e3_to_e2: false,
                    };
                    return self
                        .finish_degraded(decision, knowledge_refs, started)
                        .await;
                }
                None => continue,
            }
        }

        // 4. Chain exhausted (F_F5 §6): build the downgrade decision and,
        //    when every configured provider is now cooling, flip global
        //    offline (F_F5 §4.4).
        let went_offline = {
            let mut st = self.inner.lock().expect("fallback state lock poisoned");
            let offline = all_cooling(&registered, &st.cooldowns, self.clock.now());
            let newly = offline && !st.global_offline;
            st.global_offline = offline;
            newly
        };
        if went_offline {
            tracing::warn!(
                event = "ai_global_offline",
                "all configured ai providers are cooling or unreachable"
            );
            self.events.ai_offline("all_providers_unavailable");
        }

        let action = suggested_action_for(cap, auth_level);
        let decision = DowngradeDecision {
            account_id: account_id.to_string(),
            capability: cap,
            reason: "all_providers_unavailable".into(),
            failed_chain,
            suggested_action: action,
            retry_after: self.earliest_cooldown_expiry(self.clock.now()),
            should_downgrade_e3_to_e2: should_downgrade_e3(auth_level, action),
        };
        let hold = action == SuggestedAction::Hold;
        self.finish_degraded_with_request(
            decision,
            knowledge_refs,
            started,
            hold.then(|| (account_id.to_string(), cap, request)),
        )
        .await
    }

    /// Try one resolved provider: skip it when cooling, otherwise call with
    /// the bounded in-place retry policy and apply a cooldown on failure.
    async fn try_provider(
        &self,
        client: Arc<dyn AiProviderClient>,
        request: &ChatRequest,
        failed_chain: &mut Vec<String>,
        exclude: &mut Vec<AiProvider>,
        now: i64,
    ) -> AppResult<Option<ChainStep>> {
        let provider = client.id();
        exclude.push(provider);

        if self.is_cooling(provider, now) {
            // Cooled providers are skipped without waiting (F_F5 §4.3).
            failed_chain.push(chain_entry(provider, "cooling_down"));
            return Ok(None);
        }

        match self.attempt(&*client, request).await {
            AttemptOutcome::Success(response) => Ok(Some(ChainStep::Done(response, provider))),
            AttemptOutcome::Failed(err) => {
                match classify(&err) {
                    FailureClass::Aborted => Err(ProviderError::Canceled.into()),
                    FailureClass::PromptUser => {
                        // ContextTooLong: the prompt is the problem — switching
                        // providers would not help (card §6).
                        failed_chain.push(chain_entry(provider, failure_tag(&err)));
                        Ok(Some(ChainStep::HumanNeeded("context_too_long".into())))
                    }
                    FailureClass::RetryThenSwitch | FailureClass::SwitchImmediately => {
                        failed_chain.push(chain_entry(provider, failure_tag(&err)));
                        if let Some((until, reason)) = cooldown_after(&err, self.clock.now()) {
                            self.set_cooldown(provider, until, reason, client.clone());
                        }
                        Ok(None)
                    }
                }
            }
        }
    }

    /// One provider attempt with bounded exponential-backoff retries for
    /// transient failures (card §3: at most [`TRANSIENT_RETRY_MAX`] retries,
    /// exponential with jitter). Non-transient failures return immediately.
    async fn attempt(
        &self,
        client: &dyn AiProviderClient,
        request: &ChatRequest,
    ) -> AttemptOutcome {
        let mut attempt: u32 = 0;
        loop {
            match client.chat(request.clone()).await {
                Ok(response) => return AttemptOutcome::Success(response),
                Err(err) => {
                    let retryable = classify(&err) == FailureClass::RetryThenSwitch;
                    if !retryable || attempt >= TRANSIENT_RETRY_MAX {
                        return AttemptOutcome::Failed(err);
                    }
                    attempt += 1;
                    let backoff = RETRY_BACKOFF_BASE_MS * (1 << (attempt - 1));
                    let jitter = {
                        use rand::Rng;
                        rand::thread_rng().gen_range(0..=RETRY_JITTER_MAX_MS)
                    };
                    tracing::debug!(
                        event = "ai_fallback_retry",
                        provider = client.id().as_str(),
                        attempt = attempt,
                        "transient provider failure; retrying in place"
                    );
                    tokio::time::sleep(Duration::from_millis(backoff + jitter)).await;
                }
            }
        }
    }

    // ── Completion paths (audit row + return) ───────────────────────────────

    #[allow(clippy::too_many_arguments)]
    async fn finish_completed(
        &self,
        account_id: &str,
        cap: Capability,
        response: ChatResponse,
        provider: AiProvider,
        failed_chain: Vec<String>,
        knowledge_refs: &[String],
        started: std::time::Instant,
    ) -> AppResult<InvokeOutcome> {
        self.write_audit(
            account_id,
            cap,
            &response.model_echo,
            Some(response.usage.prompt_tokens as i64),
            Some(response.usage.completion_tokens as i64),
            started.elapsed().as_millis() as i64,
            knowledge_refs,
            &format!("completed via {}", provider.as_str()),
        )
        .await?;
        Ok(InvokeOutcome::Completed {
            response,
            provider,
            failed_chain,
        })
    }

    async fn finish_degraded(
        &self,
        decision: DowngradeDecision,
        knowledge_refs: &[String],
        started: std::time::Instant,
    ) -> AppResult<InvokeOutcome> {
        self.finish_degraded_with_request(decision, knowledge_refs, started, None)
            .await
    }

    /// Audit the degraded call (`ai_model = "downgraded:<action>"`, dev/06
    /// §9 — no prompt/completion text ever) and enqueue the held request
    /// when one is supplied.
    async fn finish_degraded_with_request(
        &self,
        decision: DowngradeDecision,
        knowledge_refs: &[String],
        started: std::time::Instant,
        hold_request: Option<(String, Capability, ChatRequest)>,
    ) -> AppResult<InvokeOutcome> {
        let placeholder = format!(
            "{DOWNGRADED_MODEL_PREFIX}{}",
            decision.suggested_action.as_str()
        );
        self.write_audit(
            &decision.account_id,
            decision.capability,
            &placeholder,
            None,
            None,
            started.elapsed().as_millis() as i64,
            knowledge_refs,
            &format!("degraded: {}", decision.reason),
        )
        .await?;

        if let Some((account_id, capability, request)) = hold_request {
            self.enqueue_hold(HoldEntry {
                account_id,
                capability,
                request,
                enqueued_at: self.clock.now(),
            });
        }

        tracing::info!(
            event = "ai_call_degraded",
            account_id = %decision.account_id,
            capability = decision.capability.as_str(),
            reason = %decision.reason,
            action = decision.suggested_action.as_str(),
            e3_downgrade = decision.should_downgrade_e3_to_e2,
            "ai invocation degraded toward human control"
        );
        Ok(InvokeOutcome::Degraded(decision))
    }

    /// Append one `ai_decisions` row (dev/06 §9). Identifiers, counts, and
    /// class tags only — never prompt or completion text (09 §5).
    #[allow(clippy::too_many_arguments)]
    async fn write_audit(
        &self,
        account_id: &str,
        cap: Capability,
        ai_model: &str,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        latency_ms: i64,
        knowledge_refs: &[String],
        result_description: &str,
    ) -> AppResult<()> {
        let refs_json = serde_json::to_string(knowledge_refs).unwrap_or_else(|_| "[]".into());
        sqlx::query(
            "INSERT INTO ai_decisions (id, account_id, decision_type, impact, \
                 action_description, knowledge_refs, result_description, ai_model, \
                 input_tokens, output_tokens, latency_ms, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(new_uuid())
        .bind(account_id)
        .bind(cap.as_str())
        .bind(impact_for(cap))
        .bind(format!("ai invocation routed for {}", cap.as_str()))
        .bind(refs_json)
        .bind(result_description)
        .bind(ai_model)
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(latency_ms)
        .bind(self.clock.now())
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    // ── Hold queue (F_F5 §4.2/§4.4) ─────────────────────────────────────────

    /// Park a task until a provider recovers. Public so module-E engines can
    /// hold their own work items through the same queue.
    pub fn enqueue_hold(&self, entry: HoldEntry) {
        self.inner
            .lock()
            .expect("fallback state lock poisoned")
            .hold
            .push(entry);
    }

    pub fn hold_len(&self) -> usize {
        self.inner
            .lock()
            .expect("fallback state lock poisoned")
            .hold
            .len()
    }

    /// Bounded, throttled catch-up after recovery (dev/06 §7): replays at
    /// most [`CATCH_UP_MAX_PER_RUN`] held tasks, [`CATCH_UP_DELAY_MS`] apart,
    /// so a backlog never stampedes a provider that just came back. Items the
    /// run could not serve degrade again (and re-queue when held). Returns
    /// each replayed entry with its outcome so future module-E callers can
    /// persist the produced drafts.
    pub async fn catch_up(&self) -> Vec<(HoldEntry, AppResult<InvokeOutcome>)> {
        let batch: Vec<HoldEntry> = {
            let mut st = self.inner.lock().expect("fallback state lock poisoned");
            let take = st.hold.len().min(CATCH_UP_MAX_PER_RUN);
            st.hold.drain(..take).collect()
        };
        let mut results = Vec::with_capacity(batch.len());
        for (i, entry) in batch.into_iter().enumerate() {
            if i > 0 {
                tokio::time::sleep(Duration::from_millis(CATCH_UP_DELAY_MS)).await;
            }
            let outcome = self
                .invoke(
                    &entry.account_id,
                    entry.capability,
                    entry.request.clone(),
                    &[],
                )
                .await;
            results.push((entry, outcome));
        }
        results
    }

    // ── Cooldown bookkeeping + recovery probes ──────────────────────────────

    fn set_cooldown(
        &self,
        provider: AiProvider,
        until: i64,
        reason: CooldownReason,
        client: Arc<dyn AiProviderClient>,
    ) {
        tracing::info!(
            event = "ai_provider_cooldown",
            provider = provider.as_str(),
            reason = reason.as_str(),
            until = until,
            "provider entered cooldown"
        );
        self.inner
            .lock()
            .expect("fallback state lock poisoned")
            .cooldowns
            .insert(
                provider,
                CooldownEntry {
                    until,
                    reason,
                    client,
                },
            );
    }

    fn is_cooling(&self, provider: AiProvider, now: i64) -> bool {
        self.inner
            .lock()
            .expect("fallback state lock poisoned")
            .cooldowns
            .get(&provider)
            .is_some_and(|e| e.is_cooling(now))
    }

    fn earliest_cooldown_expiry(&self, now: i64) -> Option<i64> {
        self.inner
            .lock()
            .expect("fallback state lock poisoned")
            .cooldowns
            .values()
            .filter(|e| e.is_cooling(now))
            .map(|e| e.until)
            .min()
    }

    pub fn is_global_offline(&self) -> bool {
        self.inner
            .lock()
            .expect("fallback state lock poisoned")
            .global_offline
    }

    /// Status snapshot for the T069 indicator panel (F_F5 §5).
    pub fn status(&self) -> AiRouterStatus {
        let st = self.inner.lock().expect("fallback state lock poisoned");
        AiRouterStatus {
            global_offline: st.global_offline,
            hold_count: st.hold.len() as u32,
            cooldowns: st
                .cooldowns
                .iter()
                .map(|(p, e)| CooldownInfo {
                    provider: *p,
                    until: e.until,
                    reason: e.reason.as_str().to_string(),
                })
                .collect(),
        }
    }

    /// One recovery pass — the bootstrap (T107) spawns this on a 60 s
    /// interval (F_F5 §4.3–§4.4):
    ///
    /// * collects every cooling provider inside its probe window
    ///   ([`PROBE_LEAD_SECS`] before expiry) — or, while globally offline,
    ///   every cooling provider regardless of window — and orders them
    ///   closest-to-expiry first with the provider id as tie-breaker, so the
    ///   pass never depends on hash-map iteration order;
    /// * probes each candidate's `health()` in that order, at most once per
    ///   tick: the first success lifts that cooldown, emits `ai:online`,
    ///   clears the global-offline state, runs one bounded
    ///   [`Self::catch_up`], and ends the pass;
    /// * every failed probe resets that provider's cooldown timer per its
    ///   original failure class — a dead candidate never starves the probe of
    ///   the next one, so a single recovered provider always brings service
    ///   back within one tick.
    pub async fn run_recovery_tick(&self) {
        let now = self.clock.now();
        let mut candidates: Vec<(AiProvider, Arc<dyn AiProviderClient>, CooldownReason, i64)> = {
            let st = self.inner.lock().expect("fallback state lock poisoned");
            st.cooldowns
                .iter()
                .filter(|(_, e)| e.is_cooling(now) && (st.global_offline || e.probe_due(now)))
                .map(|(p, e)| (*p, e.client.clone(), e.reason, e.until))
                .collect()
        };
        candidates.sort_by(|a, b| a.3.cmp(&b.3).then_with(|| a.0.as_str().cmp(b.0.as_str())));

        for (provider, client, reason, _) in candidates {
            let recovered = matches!(health_with_retry(&*client).await, Ok(h) if h.ok);
            if recovered {
                {
                    let mut st = self.inner.lock().expect("fallback state lock poisoned");
                    st.cooldowns.remove(&provider);
                    st.global_offline = false;
                }
                tracing::info!(
                    event = "ai_provider_recovered",
                    provider = provider.as_str(),
                    "cooldown lifted by recovery probe"
                );
                self.events.ai_online(provider.as_str());
                let replayed = self.catch_up().await;
                if !replayed.is_empty() {
                    tracing::info!(
                        event = "ai_hold_catch_up",
                        replayed = replayed.len(),
                        remaining = self.hold_len(),
                        "held tasks replayed after recovery"
                    );
                }
                // One recovery (and one bounded catch-up) per tick: remaining
                // candidates keep their probe windows for the next pass.
                return;
            }
            {
                let mut st = self.inner.lock().expect("fallback state lock poisoned");
                if let Some(entry) = st.cooldowns.get_mut(&provider) {
                    entry.until = self.clock.now() + reason.cooldown_secs();
                }
            }
            tracing::debug!(
                event = "ai_probe_failed",
                provider = provider.as_str(),
                reason = reason.as_str(),
                "recovery probe failed; cooldown timer reset"
            );
        }
    }

    // ── Settings reads ──────────────────────────────────────────────────────

    /// `app_settings["ai.disable_until"]` — `None` when absent or JSON null.
    async fn disabled_until(&self) -> AppResult<Option<i64>> {
        let raw = SettingRepo::new(&self.db).get(AI_DISABLE_UNTIL_KEY).await?;
        Ok(raw.and_then(|v| serde_json::from_str::<Option<i64>>(&v).ok().flatten()))
    }

    /// The account's authorization level (1 = E1 manual … 3 = E3 full-auto).
    async fn auth_level(&self, account_id: &str) -> AppResult<u8> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT auth_level FROM account_ai_settings WHERE account_id = ?")
                .bind(account_id)
                .fetch_optional(self.db.pool())
                .await
                .map_err(map_sqlx_err)?;
        let (level,) = row.ok_or(AppError::NotFound)?;
        Ok(level.clamp(1, 3) as u8)
    }
}

/// One step of the chain walk inside `invoke`.
enum ChainStep {
    Done(ChatResponse, AiProvider),
    HumanNeeded(String),
}

/// Are *all* registered providers cooling at `now`? Pure over its inputs so
/// the global-offline trigger is unit-testable.
fn all_cooling(
    registered: &[AiProvider],
    cooldowns: &HashMap<AiProvider, CooldownEntry>,
    now: i64,
) -> bool {
    !registered.is_empty()
        && registered
            .iter()
            .all(|p| cooldowns.get(p).is_some_and(|e| e.is_cooling(now)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::matrix::{CapabilityMatrix, MatrixCell, MatrixEntry, ProviderAssignment};
    use crate::ai::mock::MockProvider;
    use crate::storage::Db;
    use crate::types::ErrorCode;
    use std::sync::atomic::{AtomicI64, Ordering};

    const PROMPT: &str = "Please draft a reply about the Hartley settlement terms";

    /// These tests run on the real tokio clock. All cooldown/probe policy is
    /// driven by the injected manual [`Clock`], and the operational waits
    /// (in-place retry backoff, catch-up throttle) are compressed to a few
    /// milliseconds in test builds, so nothing here needs virtual time. A
    /// paused clock would reintroduce the PoolTimedOut race: tokio's
    /// auto-advance jumps to the sqlx acquire deadline while a connection
    /// hand-back is still in flight on the SQLite worker thread.
    async fn db() -> Db {
        let db = Db::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    /// Insert an account + ai-settings row (primary provider openai).
    async fn seed_account(db: &Db, auth_level: i64) -> String {
        let id = new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, created_at, updated_at) \
             VALUES (?, ?, 'Work', 'imap', 'slate', 'W', ?, ?)",
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
             VALUES (?, ?, 'openai', 1000, ?)",
        )
        .bind(&id)
        .bind(auth_level)
        .bind(now)
        .execute(db.pool())
        .await
        .unwrap();
        id
    }

    /// Persist a DraftReply matrix: primary openai → backup anthropic.
    async fn seed_matrix(db: &Db, account_id: &str) {
        let assignment = |provider: AiProvider| ProviderAssignment {
            provider,
            model: String::new(),
            base_url: None,
        };
        let matrix = CapabilityMatrix {
            entries: vec![MatrixEntry {
                capability: Capability::DraftReply,
                cell: MatrixCell {
                    primary: assignment(AiProvider::Openai),
                    backups: vec![assignment(AiProvider::Anthropic)],
                },
            }],
        };
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

    struct Rig {
        db: Db,
        router: FallbackRouter,
        primary: Arc<MockProvider>,
        backup: Arc<MockProvider>,
        account: String,
        clock: Arc<AtomicI64>,
        now: i64,
    }

    /// Full rig: openai primary + anthropic backup, manual clock.
    async fn rig(auth_level: i64) -> Rig {
        let db = db().await;
        let account = seed_account(&db, auth_level).await;
        seed_matrix(&db, &account).await;

        let registry = AiRegistry::new(db.clone());
        let primary = Arc::new(MockProvider::healthy(AiProvider::Openai));
        let backup = Arc::new(MockProvider::healthy(AiProvider::Anthropic));
        registry.register(primary.clone());
        registry.register(backup.clone());

        let now = now_unix();
        let (clock_impl, clock) = Clock::manual(now);
        let router = FallbackRouter::with_clock(registry, db.clone(), Emitter::noop(), clock_impl);
        Rig {
            db,
            router,
            primary,
            backup,
            account,
            clock,
            now,
        }
    }

    fn request() -> ChatRequest {
        ChatRequest::simple("mock-model", PROMPT, Capability::DraftReply)
    }

    async fn audit_rows(db: &Db, account: &str) -> Vec<(String, Option<String>, String)> {
        sqlx::query_as(
            "SELECT decision_type, ai_model, result_description FROM ai_decisions \
             WHERE account_id = ? ORDER BY rowid",
        )
        .bind(account)
        .fetch_all(db.pool())
        .await
        .unwrap()
    }

    // ── Pure decision functions ──────────────────────────────────────────────

    #[test]
    fn classification_matches_card_matrix() {
        assert_eq!(
            classify(&ProviderError::Unreachable("dns".into())),
            FailureClass::RetryThenSwitch
        );
        assert_eq!(
            classify(&ProviderError::BadResponse("503".into())),
            FailureClass::RetryThenSwitch
        );
        assert_eq!(
            classify(&ProviderError::RateLimited { retry_after: None }),
            FailureClass::RetryThenSwitch
        );
        assert_eq!(
            classify(&ProviderError::RateLimited {
                retry_after: Some(Duration::from_secs(600))
            }),
            FailureClass::SwitchImmediately
        );
        assert_eq!(
            classify(&ProviderError::Auth),
            FailureClass::SwitchImmediately
        );
        assert_eq!(
            classify(&ProviderError::ContentFiltered),
            FailureClass::SwitchImmediately
        );
        assert_eq!(
            classify(&ProviderError::ContextTooLong),
            FailureClass::PromptUser
        );
        assert_eq!(classify(&ProviderError::Canceled), FailureClass::Aborted);
    }

    #[test]
    fn cooldown_auth_sets_60min() {
        let now = 1_000_000;
        let (until, reason) = cooldown_after(&ProviderError::Auth, now).unwrap();
        assert_eq!(until, now + COOLDOWN_AUTH_SECS);
        assert_eq!(reason, CooldownReason::Auth);
    }

    #[test]
    fn cooldown_rate_limited_honors_retry_after() {
        let now = 1_000_000;
        let (until, reason) = cooldown_after(
            &ProviderError::RateLimited {
                retry_after: Some(Duration::from_secs(600)),
            },
            now,
        )
        .unwrap();
        assert_eq!(until, now + 600);
        assert_eq!(reason, CooldownReason::RateLimited);

        // No Retry-After → the 10-minute default.
        let (until, _) =
            cooldown_after(&ProviderError::RateLimited { retry_after: None }, now).unwrap();
        assert_eq!(until, now + COOLDOWN_RATE_LIMITED_DEFAULT_SECS);

        // Transient classes get the 5-minute cooldown.
        let (until, reason) =
            cooldown_after(&ProviderError::Unreachable("refused".into()), now).unwrap();
        assert_eq!(until, now + COOLDOWN_TRANSIENT_SECS);
        assert_eq!(reason, CooldownReason::Transient);

        // Content-class failures never cool the provider.
        assert!(cooldown_after(&ProviderError::ContextTooLong, now).is_none());
        assert!(cooldown_after(&ProviderError::ContentFiltered, now).is_none());
    }

    #[test]
    fn suggested_actions_always_move_toward_human_control() {
        // DraftReply: E2/E3 hold for review; E1 prompts the user.
        assert_eq!(
            suggested_action_for(Capability::DraftReply, 1),
            SuggestedAction::PromptUser
        );
        assert_eq!(
            suggested_action_for(Capability::DraftReply, 2),
            SuggestedAction::Hold
        );
        assert_eq!(
            suggested_action_for(Capability::DraftReply, 3),
            SuggestedAction::Hold
        );
        assert_eq!(
            suggested_action_for(Capability::RiskReason, 3),
            SuggestedAction::Skip
        );
        assert_eq!(
            suggested_action_for(Capability::Summarize, 1),
            SuggestedAction::PromptUser
        );
        assert_eq!(
            suggested_action_for(Capability::StyleProfile, 2),
            SuggestedAction::Hold
        );
        // The E3→E2 flag is exactly auth 3 + hold (card §6).
        assert!(should_downgrade_e3(3, SuggestedAction::Hold));
        assert!(!should_downgrade_e3(2, SuggestedAction::Hold));
        assert!(!should_downgrade_e3(3, SuggestedAction::Skip));
    }

    // ── Chain traversal ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn fallback_switches_backup_on_unreachable() {
        let r = rig(2).await;
        r.primary
            .set_default_chat_error(ProviderError::Unreachable("connect refused".into()));

        let outcome = r
            .router
            .invoke(&r.account, Capability::DraftReply, request(), &[])
            .await
            .unwrap();
        match outcome {
            InvokeOutcome::Completed {
                provider,
                failed_chain,
                response,
            } => {
                assert_eq!(provider, AiProvider::Anthropic);
                assert_eq!(response.text, "scripted mock completion");
                assert!(failed_chain.iter().any(|e| e.contains("openai")));
            }
            InvokeOutcome::Degraded(d) => panic!("unexpected degrade: {d:?}"),
        }
        // 1 initial + 2 in-place retries on the primary, then 1 backup call.
        assert_eq!(r.primary.chat_call_count(), 1 + TRANSIENT_RETRY_MAX);
        assert_eq!(r.backup.chat_call_count(), 1);
    }

    #[tokio::test]
    async fn fallback_all_fail_returns_downgrade() {
        let r = rig(2).await;
        r.primary
            .set_default_chat_error(ProviderError::Unreachable("down".into()));
        r.backup
            .set_default_chat_error(ProviderError::Unreachable("down".into()));

        let outcome = r
            .router
            .invoke(&r.account, Capability::DraftReply, request(), &[])
            .await
            .unwrap();
        let InvokeOutcome::Degraded(decision) = outcome else {
            panic!("expected degrade");
        };
        assert_eq!(decision.reason, "all_providers_unavailable");
        assert_eq!(decision.suggested_action, SuggestedAction::Hold);
        assert_eq!(decision.failed_chain.len(), 2);
        assert!(decision.retry_after.is_some());
        // The E2 task is parked for recovery, never dropped.
        assert_eq!(r.router.hold_len(), 1);
    }

    #[tokio::test]
    async fn e3_downgrade_flag_set_and_never_skip_or_send() {
        let r = rig(3).await;
        r.primary
            .set_default_chat_error(ProviderError::Unreachable("down".into()));
        r.backup
            .set_default_chat_error(ProviderError::Unreachable("down".into()));

        let outcome = r
            .router
            .invoke(&r.account, Capability::DraftReply, request(), &[])
            .await
            .unwrap();
        let InvokeOutcome::Degraded(decision) = outcome else {
            panic!("expected degrade");
        };
        // Golden rule (dev/06 §7): E3 falls back to human review — the action
        // is hold, never skip, and there is no "send" action at all.
        assert!(decision.should_downgrade_e3_to_e2);
        assert_eq!(decision.suggested_action, SuggestedAction::Hold);
        assert_eq!(r.router.hold_len(), 1);
    }

    #[tokio::test]
    async fn auth_failure_cools_60min_without_retry() {
        let r = rig(2).await;
        r.primary.push_chat(Err(ProviderError::Auth));

        let outcome = r
            .router
            .invoke(&r.account, Capability::DraftReply, request(), &[])
            .await
            .unwrap();
        assert!(
            matches!(outcome, InvokeOutcome::Completed { provider, .. } if provider == AiProvider::Anthropic)
        );
        // Auth is deterministic: exactly one attempt, no in-place retry.
        assert_eq!(r.primary.chat_call_count(), 1);

        let status = r.router.status();
        let cd = status
            .cooldowns
            .iter()
            .find(|c| c.provider == AiProvider::Openai)
            .expect("primary must be cooling");
        assert_eq!(cd.until, r.now + COOLDOWN_AUTH_SECS);
        assert_eq!(cd.reason, "auth");
    }

    #[tokio::test]
    async fn cooldown_skips_provider_and_prevents_hammering() {
        let r = rig(2).await;
        r.primary.push_chat(Err(ProviderError::Auth));

        // First call cools the primary; backup serves it.
        r.router
            .invoke(&r.account, Capability::DraftReply, request(), &[])
            .await
            .unwrap();
        assert_eq!(r.primary.chat_call_count(), 1);

        // Second call: the cooled primary is skipped without any network call.
        let outcome = r
            .router
            .invoke(&r.account, Capability::DraftReply, request(), &[])
            .await
            .unwrap();
        let InvokeOutcome::Completed {
            provider,
            failed_chain,
            ..
        } = outcome
        else {
            panic!("expected completion via backup");
        };
        assert_eq!(provider, AiProvider::Anthropic);
        assert!(failed_chain.iter().any(|e| e.contains("cooling_down")));
        assert_eq!(
            r.primary.chat_call_count(),
            1,
            "cooled provider must not be called"
        );
        assert_eq!(r.backup.chat_call_count(), 2);
    }

    #[tokio::test]
    async fn rate_limited_retry_after_honored_in_cooldown_expiry() {
        let r = rig(2).await;
        r.primary.push_chat(Err(ProviderError::RateLimited {
            retry_after: Some(Duration::from_secs(600)),
        }));

        // Backup takes over immediately (no in-place retry for 429+Retry-After).
        let outcome = r
            .router
            .invoke(&r.account, Capability::DraftReply, request(), &[])
            .await
            .unwrap();
        assert!(
            matches!(outcome, InvokeOutcome::Completed { provider, .. } if provider == AiProvider::Anthropic)
        );
        assert_eq!(r.primary.chat_call_count(), 1);

        let status = r.router.status();
        let cd = status
            .cooldowns
            .iter()
            .find(|c| c.provider == AiProvider::Openai)
            .unwrap();
        assert_eq!(cd.until, r.now + 600);

        // Advance past the Retry-After window: the primary is eligible again.
        r.clock.store(r.now + 601, Ordering::SeqCst);
        let outcome = r
            .router
            .invoke(&r.account, Capability::DraftReply, request(), &[])
            .await
            .unwrap();
        assert!(
            matches!(outcome, InvokeOutcome::Completed { provider, .. } if provider == AiProvider::Openai)
        );
        assert_eq!(r.primary.chat_call_count(), 2);
    }

    #[tokio::test]
    async fn context_too_long_does_not_switch_backup() {
        let r = rig(2).await;
        r.primary.push_chat(Err(ProviderError::ContextTooLong));

        let outcome = r
            .router
            .invoke(&r.account, Capability::DraftReply, request(), &[])
            .await
            .unwrap();
        let InvokeOutcome::Degraded(decision) = outcome else {
            panic!("expected degrade");
        };
        assert_eq!(decision.reason, "context_too_long");
        assert_eq!(decision.suggested_action, SuggestedAction::PromptUser);
        // The backup must not be tried — the prompt is the problem (card §6).
        assert_eq!(r.backup.chat_call_count(), 0);
        // And the primary is not cooled: it is healthy, the prompt is not.
        assert!(r.router.status().cooldowns.is_empty());
    }

    // ── User-forced degradation (F_F5 §4.5) ─────────────────────────────────

    #[tokio::test]
    async fn user_disabled_skips_all_network() {
        let r = rig(2).await;
        SettingRepo::new(&r.db)
            .set(AI_DISABLE_UNTIL_KEY, &(r.now + 86_400).to_string())
            .await
            .unwrap();

        let outcome = r
            .router
            .invoke(&r.account, Capability::DraftReply, request(), &[])
            .await
            .unwrap();
        let InvokeOutcome::Degraded(decision) = outcome else {
            panic!("expected degrade");
        };
        assert_eq!(decision.reason, "user_disabled");
        assert_eq!(decision.retry_after, Some(r.now + 86_400));
        assert_eq!(r.primary.chat_call_count(), 0);
        assert_eq!(r.backup.chat_call_count(), 0);

        // Clearing the key (JSON null) restores service immediately.
        SettingRepo::new(&r.db)
            .set(AI_DISABLE_UNTIL_KEY, "null")
            .await
            .unwrap();
        let outcome = r
            .router
            .invoke(&r.account, Capability::DraftReply, request(), &[])
            .await
            .unwrap();
        assert!(matches!(outcome, InvokeOutcome::Completed { .. }));
    }

    // ── Global offline + recovery (F_F5 §4.3–§4.4) ──────────────────────────

    #[tokio::test]
    async fn global_offline_short_circuits_without_network() {
        let r = rig(2).await;
        r.primary
            .set_default_chat_error(ProviderError::Unreachable("down".into()));
        r.backup
            .set_default_chat_error(ProviderError::Unreachable("down".into()));

        // First call walks the whole chain and flips global offline.
        r.router
            .invoke(&r.account, Capability::DraftReply, request(), &[])
            .await
            .unwrap();
        assert!(r.router.is_global_offline());
        let primary_calls = r.primary.chat_call_count();
        let backup_calls = r.backup.chat_call_count();

        // While offline, calls degrade instantly — zero further network.
        let outcome = r
            .router
            .invoke(&r.account, Capability::DraftReply, request(), &[])
            .await
            .unwrap();
        let InvokeOutcome::Degraded(decision) = outcome else {
            panic!("expected degrade");
        };
        assert_eq!(decision.reason, "all_providers_unavailable");
        assert_eq!(r.primary.chat_call_count(), primary_calls);
        assert_eq!(r.backup.chat_call_count(), backup_calls);
        // Both held tasks are queued for recovery.
        assert_eq!(r.router.hold_len(), 2);
    }

    #[tokio::test]
    async fn probe_fires_inside_lead_window_and_lifts_cooldown() {
        let r = rig(2).await;
        r.primary.push_chat(Err(ProviderError::RateLimited {
            retry_after: Some(Duration::from_secs(600)),
        }));
        r.router
            .invoke(&r.account, Capability::DraftReply, request(), &[])
            .await
            .unwrap();
        assert_eq!(r.router.status().cooldowns.len(), 1);

        // Far from expiry → no probe runs.
        r.clock.store(r.now + 100, Ordering::SeqCst);
        r.router.run_recovery_tick().await;
        assert_eq!(r.primary.health_call_count(), 0);
        assert_eq!(r.router.status().cooldowns.len(), 1);

        // Inside the 60 s pre-expiry window → probe fires; mock is healthy,
        // so the cooldown lifts early.
        r.clock.store(r.now + 600 - 45, Ordering::SeqCst);
        r.router.run_recovery_tick().await;
        assert!(r.primary.health_call_count() >= 1);
        assert!(r.router.status().cooldowns.is_empty());
    }

    #[tokio::test]
    async fn failed_probe_resets_cooldown_timer() {
        let r = rig(2).await;
        r.primary
            .set_default_chat_error(ProviderError::Unreachable("down".into()));
        r.backup.push_chat(Ok(ChatResponse {
            text: "ok".into(),
            finish: crate::ai::types::FinishReason::Stop,
            usage: Default::default(),
            model_echo: "mock-model".into(),
            latency_ms: 1,
        }));
        r.router
            .invoke(&r.account, Capability::DraftReply, request(), &[])
            .await
            .unwrap();

        // Move inside the probe window and make the probe fail (twice — the
        // health helper retries once on Unreachable).
        r.primary
            .push_health(Err(ProviderError::Unreachable("still down".into())));
        r.primary
            .push_health(Err(ProviderError::Unreachable("still down".into())));
        let probe_at = r.now + COOLDOWN_TRANSIENT_SECS - 30;
        r.clock.store(probe_at, Ordering::SeqCst);
        r.router.run_recovery_tick().await;

        let status = r.router.status();
        let cd = status
            .cooldowns
            .iter()
            .find(|c| c.provider == AiProvider::Openai)
            .expect("cooldown must persist after a failed probe");
        // Timer reset from the probe instant, per the original failure class.
        assert_eq!(cd.until, probe_at + COOLDOWN_TRANSIENT_SECS);
    }

    #[tokio::test]
    async fn recovery_drains_hold_queue_bounded() {
        let r = rig(2).await;
        // Park more entries than one catch-up run may replay.
        for _ in 0..(CATCH_UP_MAX_PER_RUN + 2) {
            r.router.enqueue_hold(HoldEntry {
                account_id: r.account.clone(),
                capability: Capability::DraftReply,
                request: request(),
                enqueued_at: r.now,
            });
        }

        let results = r.router.catch_up().await;
        assert_eq!(results.len(), CATCH_UP_MAX_PER_RUN);
        assert!(results
            .iter()
            .all(|(_, outcome)| matches!(outcome, Ok(InvokeOutcome::Completed { .. }))));
        // Bounded: the overflow stays queued for the next pass (dev/06 §7).
        assert_eq!(r.router.hold_len(), 2);
        assert_eq!(
            r.primary.chat_call_count() as usize,
            CATCH_UP_MAX_PER_RUN,
            "throttled catch-up replays exactly one bounded batch"
        );
    }

    #[tokio::test]
    async fn recovery_tick_probes_while_globally_offline_and_catches_up() {
        let r = rig(2).await;
        r.primary
            .set_default_chat_error(ProviderError::Unreachable("down".into()));
        r.backup
            .set_default_chat_error(ProviderError::Unreachable("down".into()));
        r.router
            .invoke(&r.account, Capability::DraftReply, request(), &[])
            .await
            .unwrap();
        assert!(r.router.is_global_offline());
        assert_eq!(r.router.hold_len(), 1);

        // The primary comes back: lift its scripted failure so both the
        // health probe and the replayed chat succeed. The backup is still
        // down, so its probe must fail too (`BadResponse` is probed exactly
        // once — no health retry). Both cooldowns expire at the same instant,
        // and the tick walks the candidates in deterministic order until one
        // truly recovers, so the result never depends on map order.
        r.primary.clear_default_chat_error();
        r.backup
            .push_health(Err(ProviderError::BadResponse("503".into())));
        // Globally offline → the tick probes even far from cooldown expiry.
        r.clock.store(r.now + 10, Ordering::SeqCst);
        r.router.run_recovery_tick().await;

        assert!(!r.router.is_global_offline());
        assert!(r.primary.health_call_count() >= 1);
        // The held task was replayed and completed on the recovered provider.
        assert_eq!(r.router.hold_len(), 0);
        // The still-down backup keeps cooling — only a passed probe lifts it.
        assert!(r
            .router
            .status()
            .cooldowns
            .iter()
            .any(|c| c.provider == AiProvider::Anthropic));
    }

    // ── E7 audit rows (dev/06 §9, 09 §5) ────────────────────────────────────

    #[tokio::test]
    async fn ai_decisions_written_for_success_and_degrade() {
        let r = rig(2).await;
        // Success row.
        r.router
            .invoke(
                &r.account,
                Capability::DraftReply,
                request(),
                &["mail:abc".into()],
            )
            .await
            .unwrap();
        // Degraded row.
        r.primary
            .set_default_chat_error(ProviderError::Unreachable("down".into()));
        r.backup
            .set_default_chat_error(ProviderError::Unreachable("down".into()));
        r.router
            .invoke(&r.account, Capability::DraftReply, request(), &[])
            .await
            .unwrap();

        let rows = audit_rows(&r.db, &r.account).await;
        assert_eq!(rows.len(), 2);
        let (decision_type, model, result) = &rows[0];
        assert_eq!(decision_type, "draft_reply");
        assert_eq!(model.as_deref(), Some("mock-model"));
        assert!(result.contains("completed via openai"));

        let (_, model, result) = &rows[1];
        assert_eq!(model.as_deref(), Some("downgraded:hold"));
        assert!(result.contains("all_providers_unavailable"));
    }

    #[tokio::test]
    async fn audit_rows_never_contain_prompt_text() {
        let r = rig(2).await;
        r.router
            .invoke(&r.account, Capability::DraftReply, request(), &[])
            .await
            .unwrap();
        r.primary
            .set_default_chat_error(ProviderError::Unreachable("down".into()));
        r.backup
            .set_default_chat_error(ProviderError::Unreachable("down".into()));
        r.router
            .invoke(&r.account, Capability::DraftReply, request(), &[])
            .await
            .unwrap();

        // Log-safety (09 §5): no column of any row carries prompt/completion
        // text — only identifiers and class tags.
        let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT action_description, result_description, ai_model FROM ai_decisions \
             WHERE account_id = ?",
        )
        .bind(&r.account)
        .fetch_all(r.db.pool())
        .await
        .unwrap();
        assert!(!rows.is_empty());
        for (action, result, model) in rows {
            assert!(!action.contains("Hartley"));
            assert!(!result.contains("Hartley"));
            assert!(!result.contains("scripted mock completion"));
            assert!(!model.unwrap_or_default().contains("Hartley"));
        }
    }

    #[tokio::test]
    async fn not_configured_prompts_user() {
        let db = db().await;
        let account = seed_account(&db, 2).await;
        sqlx::query("UPDATE account_ai_settings SET ai_provider = 'none' WHERE account_id = ?")
            .bind(&account)
            .execute(db.pool())
            .await
            .unwrap();
        let registry = AiRegistry::new(db.clone());
        let router = FallbackRouter::new(registry, db, Emitter::noop());

        let outcome = router
            .invoke(&account, Capability::DraftReply, request(), &[])
            .await
            .unwrap();
        let InvokeOutcome::Degraded(decision) = outcome else {
            panic!("expected degrade");
        };
        assert_eq!(decision.reason, "not_configured");
        assert_eq!(decision.suggested_action, SuggestedAction::PromptUser);
        assert!(!decision.should_downgrade_e3_to_e2);
    }

    #[tokio::test]
    async fn unknown_account_is_a_hard_error_not_a_degrade() {
        let db = db().await;
        let registry = AiRegistry::new(db.clone());
        let router = FallbackRouter::new(registry, db, Emitter::noop());
        let err = router
            .invoke("missing-account", Capability::DraftReply, request(), &[])
            .await
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::NotFound);
    }
}
