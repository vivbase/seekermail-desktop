//! E5 style learning (T075, F_E5) — sent-mail sampling, two-phase LLM style
//! profiling, persistence, and the 30-day refresh worker.
//!
//! Pipeline (`run_style_learning`):
//!
//! 1. [`sampler::sample_sent_mails`] — qualifying sent mails (last 180 days,
//!    30–2000 chars, no `Fwd:`/auto-reply chains, < 10 recipients, ≤ 200 kept).
//! 2. [`profiler::build_style_profile`] — two-stage `chat` calls via the F4
//!    route for [`Capability::StyleProfile`]: per-group partial summaries, then
//!    one merge call producing the six-dimension summary (F_E5 §4.2/§4.3).
//! 3. [`repo::save_style_profile`] — JSON into `account_ai_settings.style_profile`
//!    plus `style_samples_count`; a user-pinned profile is never overwritten
//!    (F_E5 §4.5).
//!
//! Cold start (AI_MODES §6.7): fewer than [`MIN_SAMPLES`] qualifying mails →
//! no profile is written and the failure surfaces as `VALIDATION` via
//! `style:error`; draft generation falls back to templates until enough
//! history accumulates.
//!
//! Log safety (09 §5): every log line in this module family carries
//! identifiers, counts, stages, and latencies only — never mail bodies,
//! subjects, sender addresses, or prompt content.

pub mod injector;
pub mod profiler;
pub mod repo;
pub mod sampler;
pub mod validator;

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use once_cell::sync::Lazy;

use crate::ai::types::Capability;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage::{map_sqlx_err, Db};
use crate::util::now_unix;

pub use injector::build_style_block;
pub use profiler::{
    build_style_profile, StyleDonePayload, StyleErrorPayload, StyleProfileJson,
    StyleProgressPayload, StyleSummary,
};
pub use repo::{load_style_profile, save_style_profile};
pub use sampler::{sample_sent_mails, StyleSample};
pub use validator::{check_style_drift, StyleDriftResult};

// ── Tuning constants (F_E5 §4.1–§4.3, T075 §3) ───────────────────────────────

/// Cold-start floor: below this many qualifying sent mails no profile is built
/// (AI_MODES §6.7).
pub const MIN_SAMPLES: usize = 20;
/// Hard cap on sampled mails; beyond it the corpus is uniformly thinned.
pub const MAX_SAMPLES: usize = 200;
/// Sampling window — sent mails from the last 180 days.
pub const SAMPLE_WINDOW_SECS: i64 = 180 * 86_400;
/// Qualifying body length bounds (Unicode chars).
pub const BODY_MIN_CHARS: usize = 30;
pub const BODY_MAX_CHARS: usize = 2_000;
/// Per-sample body cap fed to the profiler (token control, T075 §3).
pub const BODY_TRIM_CHARS: usize = 500;
/// Group sends (≥ this many `to` recipients) are excluded.
pub const MAX_RECIPIENTS: usize = 10;
/// Stage-1 grouping: one `chat` call per this many samples.
pub const GROUP_SIZE: usize = 20;
/// Schema version of the persisted profile JSON (F_E5 §4.2).
pub const STYLE_PROFILE_VERSION: u32 = 1;
/// Locally kept snippets (never injected into prompts, F_E5 §4.4).
pub const SNIPPET_COUNT: usize = 3;
pub const SNIPPET_MAX_CHARS: usize = 100;
/// Automatic recompute cadence (F_E5 §4.3).
pub const REFRESH_INTERVAL_SECS: i64 = 30 * 86_400;
/// How often the refresh worker re-checks for due accounts. The first check
/// fires immediately at startup (T075 §3 scheduled refresh).
const REFRESH_CHECK_PERIOD_SECS: u64 = 6 * 3_600;

/// `style:progress` stage tags (T075 §3 IPC events).
pub const STAGE_SAMPLING: &str = "sampling";
pub const STAGE_PROFILING: &str = "profiling";
pub const STAGE_DONE: &str = "done";

// ── Single-flight guard ───────────────────────────────────────────────────────

/// Accounts with a style run currently in flight. A duplicate trigger is a
/// no-op: the running task's event stream already covers the caller (T075 §6).
static IN_FLIGHT: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));

/// Try to claim the single-flight slot for one account. `false` = already running.
pub(crate) fn begin_run(account_id: &str) -> bool {
    IN_FLIGHT
        .lock()
        .expect("style in-flight guard poisoned")
        .insert(account_id.to_string())
}

/// Release the single-flight slot.
pub(crate) fn end_run(account_id: &str) {
    IN_FLIGHT
        .lock()
        .expect("style in-flight guard poisoned")
        .remove(account_id);
}

// ── Pipeline ─────────────────────────────────────────────────────────────────

/// The full learning pass for one account: sample → profile → persist, with
/// `style:progress` emitted once per stage. Returns the sample count.
///
/// Below [`MIN_SAMPLES`] qualifying mails this returns a `VALIDATION` error and
/// leaves `style_profile` untouched (AI_MODES §6.7 cold start).
pub async fn run_style_learning(state: &AppState, account_id: &str) -> AppResult<i64> {
    let started = std::time::Instant::now();
    state.events.style_progress(account_id, STAGE_SAMPLING, 10);

    let samples = sampler::sample_sent_mails(state.storage.db(), account_id).await?;
    let sample_count = samples.len() as i64;
    if samples.len() < MIN_SAMPLES {
        return Err(AppError::Validation(format!(
            "insufficient sent mail samples for style learning: {sample_count} found, {MIN_SAMPLES} required"
        )));
    }

    // F4 routing: per-capability provider resolution + daily-limit guardrail.
    let client = state
        .ai
        .resolve(account_id, Capability::StyleProfile)
        .await?;
    let model = state
        .ai
        .account_config(account_id)
        .await?
        .model
        .unwrap_or_default();

    state.events.style_progress(account_id, STAGE_PROFILING, 45);
    let profile =
        profiler::build_style_profile(account_id, &model, &samples, client.as_ref()).await?;
    repo::save_style_profile(state.storage.db(), account_id, &profile, sample_count).await?;

    state.events.style_progress(account_id, STAGE_DONE, 100);
    state.events.style_done(account_id, sample_count);
    tracing::info!(
        event = "style_learning_complete",
        account_id = account_id,
        sample_count = sample_count,
        latency_ms = started.elapsed().as_millis() as u64,
        "style profile updated"
    );
    Ok(sample_count)
}

/// Fire-and-forget launcher used by the IPC command and the refresh worker.
/// Deduplicates per account; failures surface as `style:error` (T075 §3).
pub fn trigger_style_learning_task(state: AppState, account_id: String) {
    if !begin_run(&account_id) {
        tracing::info!(
            event = "style_learning_already_running",
            account_id = %account_id,
            "duplicate style trigger ignored; the running task's events cover it"
        );
        return;
    }
    tauri::async_runtime::spawn(async move {
        let result = run_style_learning(&state, &account_id).await;
        end_run(&account_id);
        if let Err(e) = result {
            tracing::warn!(
                event = "style_learning_failed",
                account_id = %account_id,
                code = e.code().as_wire(),
                "style learning did not complete"
            );
            state.events.style_error(&account_id, e.code());
        }
    });
}

// ── 30-day refresh worker ─────────────────────────────────────────────────────

/// Whether a stored profile is due for automatic recompute.
///
/// * `None` (never learned) → **not** due: the first run is user-triggered
///   (F_E5 §4.3 enablement consent).
/// * `pinned: true` (user-edited) → not due: a recompute could never land
///   anyway (F_E5 §4.5), so no tokens are spent on it.
/// * `generated_at` missing/unreadable → due (recompute heals the record).
/// * Otherwise due once `generated_at` is ≥ 30 days old.
pub fn is_refresh_due(profile: Option<&serde_json::Value>, now: i64) -> bool {
    let Some(p) = profile else { return false };
    if p.get("pinned").and_then(|v| v.as_bool()).unwrap_or(false) {
        return false;
    }
    match p.get("generated_at").and_then(|v| v.as_i64()) {
        Some(ts) => now - ts >= REFRESH_INTERVAL_SECS,
        None => true,
    }
}

/// Active, AI-enabled accounts whose stored profile is due per
/// [`is_refresh_due`]. A profile that exists but no longer parses as JSON is
/// also due — recomputing replaces the corrupt record.
pub async fn due_account_ids(db: &Db, now: i64) -> AppResult<Vec<String>> {
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT s.account_id, s.style_profile FROM account_ai_settings s \
         JOIN accounts a ON a.id = s.account_id \
         WHERE a.is_active = 1 AND s.ai_provider != 'none'",
    )
    .fetch_all(db.pool())
    .await
    .map_err(map_sqlx_err)?;

    Ok(rows
        .into_iter()
        .filter_map(|(id, raw)| match raw {
            None => None, // never learned — first run is user-triggered
            Some(r) => match serde_json::from_str::<serde_json::Value>(&r) {
                Ok(v) => is_refresh_due(Some(&v), now).then_some(id),
                Err(_) => Some(id), // unreadable profile → recompute
            },
        })
        .collect())
}

/// Spawn the background refresh loop (called once from `lib.rs` at startup).
///
/// The first check runs immediately — covering the card's "check
/// `generated_at` at app start" requirement — then every
/// [`REFRESH_CHECK_PERIOD_SECS`]; each due account goes through the same
/// single-flight [`trigger_style_learning_task`] path as a manual trigger.
pub fn start_refresh_worker(state: AppState) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(REFRESH_CHECK_PERIOD_SECS));
        loop {
            ticker.tick().await; // first tick fires immediately
            match due_account_ids(state.storage.db(), now_unix()).await {
                Ok(ids) => {
                    if !ids.is_empty() {
                        tracing::info!(
                            event = "style_refresh_due",
                            account_count = ids.len(),
                            "queueing 30-day style profile refresh"
                        );
                    }
                    for id in ids {
                        trigger_style_learning_task(state.clone(), id);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        event = "style_refresh_scan_failed",
                        code = e.code().as_wire(),
                        "style refresh scan failed; retrying next period"
                    );
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::mock::MockProvider;
    use crate::ai::types::{ChatResponse, FinishReason, TokenUsage};
    use crate::types::{AiProvider, ErrorCode};
    use crate::util::{new_uuid, now_unix};
    use std::sync::Arc;

    /// Insert a minimal account + ai-settings row configured for `provider`.
    async fn seed_account(state: &AppState, provider: &str) -> String {
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
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO account_ai_settings (account_id, auth_level, ai_provider, ai_model, updated_at) \
             VALUES (?, 1, ?, 'gpt-4o', ?)",
        )
        .bind(&id)
        .bind(provider)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        id
    }

    /// Insert `n` qualifying sent mails for the account, dated within the window.
    async fn seed_sent_mails(state: &AppState, account_id: &str, n: usize) {
        let base = now_unix() - 30 * 86_400;
        for i in 0..n {
            sqlx::query(
                "INSERT INTO mails (id, account_id, message_id, subject, from_email, to_addrs, \
                 date_sent, date_received, body_text, is_sent, folder, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, 'me@example.com', \
                 '[{\"name\":null,\"email\":\"daniel@vendorco.example\"}]', ?, ?, ?, 1, 'Sent', 0, 0)",
            )
            .bind(format!("{account_id}-sent-{i}"))
            .bind(account_id)
            .bind(format!("<sent-{i}@{account_id}>"))
            .bind(format!("Re: Vendor contract update {i}"))
            .bind(base + i as i64 * 3_600)
            .bind(base + i as i64 * 3_600)
            .bind(format!(
                "Hi Daniel,\n\nThanks for sending over the revised statement of work. \
                 The payment schedule now matches what we agreed on the call. \
                 Could you turn a clean version around by Thursday? (Ref {i})\n\nBest regards,\nMaya"
            ))
            .execute(state.storage.db().pool())
            .await
            .unwrap();
        }
    }

    fn summary_json() -> String {
        r#"{"overall_tone":"Warm but direct; gets to the point within two sentences.","opening_patterns":["Hi {name},","Thanks for the quick turnaround","Hope your week is going well"],"closing_patterns":["Best regards,","Let me know if anything is unclear.","Talk soon,"],"sentence_length":"12-18 words on average","vocabulary":"Plain business English with contract terms such as SOW and redline","format_habit":"Short paragraphs of one to three sentences; bullet lists for action items."}"#
            .to_string()
    }

    fn ok_response(text: String) -> Result<ChatResponse, crate::ai::ProviderError> {
        Ok(ChatResponse {
            text,
            finish: FinishReason::Stop,
            usage: TokenUsage::default(),
            model_echo: "mock-model".into(),
            latency_ms: 1,
        })
    }

    // ── single-flight guard ──────────────────────────────────────────────────

    #[test]
    fn begin_run_is_single_flight_per_account() {
        let id = format!("guard-{}", new_uuid());
        assert!(begin_run(&id));
        assert!(!begin_run(&id), "second claim must be refused");
        end_run(&id);
        assert!(begin_run(&id), "slot reopens after end_run");
        end_run(&id);
    }

    // ── is_refresh_due (pure) ────────────────────────────────────────────────

    #[test]
    fn refresh_due_logic() {
        let now = now_unix();
        // Never learned → user-triggered only.
        assert!(!is_refresh_due(None, now));
        // Fresh profile → not due.
        let fresh = serde_json::json!({ "generated_at": now - 86_400 });
        assert!(!is_refresh_due(Some(&fresh), now));
        // 31 days old → due.
        let stale = serde_json::json!({ "generated_at": now - 31 * 86_400 });
        assert!(is_refresh_due(Some(&stale), now));
        // Exactly 30 days → due (inclusive boundary).
        let edge = serde_json::json!({ "generated_at": now - REFRESH_INTERVAL_SECS });
        assert!(is_refresh_due(Some(&edge), now));
        // Pinned (user-edited) → never auto-recomputed.
        let pinned = serde_json::json!({ "generated_at": now - 90 * 86_400, "pinned": true });
        assert!(!is_refresh_due(Some(&pinned), now));
        // Missing/unreadable generated_at → due (recompute heals).
        let no_ts = serde_json::json!({ "summary": {} });
        assert!(is_refresh_due(Some(&no_ts), now));
    }

    /// Overwrite one account's stored profile JSON directly.
    async fn set_profile(db: &Db, account_id: &str, json: &str) {
        sqlx::query("UPDATE account_ai_settings SET style_profile = ? WHERE account_id = ?")
            .bind(json)
            .bind(account_id)
            .execute(db.pool())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn due_account_ids_picks_only_stale_unpinned_ai_accounts() {
        let (state, _rx) = AppState::test_state().await;
        let db = state.storage.db();
        let now = now_unix();

        let stale = seed_account(&state, "openai").await;
        let fresh = seed_account(&state, "openai").await;
        let pinned = seed_account(&state, "openai").await;
        let no_ai = seed_account(&state, "none").await;
        let never = seed_account(&state, "openai").await; // style_profile stays NULL

        let stale_ts = now - 40 * 86_400;
        set_profile(db, &stale, &format!(r#"{{"generated_at": {stale_ts}}}"#)).await;
        let fresh_ts = now - 86_400;
        set_profile(db, &fresh, &format!(r#"{{"generated_at": {fresh_ts}}}"#)).await;
        let pinned_json = format!(r#"{{"generated_at": {stale_ts}, "pinned": true}}"#);
        set_profile(db, &pinned, &pinned_json).await;
        set_profile(db, &no_ai, &format!(r#"{{"generated_at": {stale_ts}}}"#)).await;

        let due = due_account_ids(db, now).await.unwrap();
        assert_eq!(due, vec![stale]);
        let _ = never;
    }

    #[tokio::test]
    async fn corrupt_stored_profile_is_due_for_recompute() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state, "openai").await;
        sqlx::query(
            "UPDATE account_ai_settings SET style_profile = 'not json' WHERE account_id = ?",
        )
        .bind(&account)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        let due = due_account_ids(state.storage.db(), now_unix())
            .await
            .unwrap();
        assert_eq!(due, vec![account]);
    }

    // ── full pipeline ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn run_style_learning_persists_profile_and_count() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state, "openai").await;
        seed_sent_mails(&state, &account, 25).await;

        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        // 25 samples → two stage-1 groups (20 + 5) + one stage-2 merge.
        mock.push_chat(ok_response(summary_json()));
        mock.push_chat(ok_response(summary_json()));
        mock.push_chat(ok_response(summary_json()));
        state.ai.register(mock.clone());

        let count = run_style_learning(&state, &account).await.unwrap();
        assert_eq!(count, 25);
        assert_eq!(mock.chat_call_count(), 3);

        let (profile, samples): (Option<String>, i64) = sqlx::query_as(
            "SELECT style_profile, style_samples_count FROM account_ai_settings WHERE account_id = ?",
        )
        .bind(&account)
        .fetch_one(state.storage.db().pool())
        .await
        .unwrap();
        assert_eq!(samples, 25);
        let v: serde_json::Value = serde_json::from_str(&profile.unwrap()).unwrap();
        assert_eq!(v["version"], 1);
        assert_eq!(v["account_id"], serde_json::Value::String(account.clone()));
        assert!(v["generated_at"].as_i64().unwrap() > 0);
        for field in [
            "overall_tone",
            "opening_patterns",
            "closing_patterns",
            "sentence_length",
            "vocabulary",
            "format_habit",
        ] {
            assert!(
                !v["summary"][field].is_null(),
                "missing summary field {field}"
            );
        }
        // After a successful run the account is no longer due.
        assert!(due_account_ids(state.storage.db(), now_unix())
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn below_min_samples_is_validation_and_writes_nothing() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state, "openai").await;
        seed_sent_mails(&state, &account, 5).await;
        state
            .ai
            .register(Arc::new(MockProvider::healthy(AiProvider::Openai)));

        let err = run_style_learning(&state, &account).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::Validation);

        let (profile, samples): (Option<String>, i64) = sqlx::query_as(
            "SELECT style_profile, style_samples_count FROM account_ai_settings WHERE account_id = ?",
        )
        .bind(&account)
        .fetch_one(state.storage.db().pool())
        .await
        .unwrap();
        assert!(profile.is_none(), "profile must stay unset below the floor");
        assert_eq!(samples, 0);
    }
}
