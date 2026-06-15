//! Data-flow disclosure commands (T069) — the AI section of the Settings →
//! Data → Data Flow panel.
//!
//! `get_data_flow_ai_routing` reports, per account, where mail content goes
//! when AI runs: the configured provider and its *real* effective endpoint
//! (dev/06 §8, ADR-0004). Cloud providers resolve to the user-configured
//! `ai_base_url` or the adapter's documented default — never to any SeekerMail
//! address, because no SeekerMail server exists in the path. Local providers
//! resolve to `localhost` (Ollama) or to no endpoint at all (in-process ONNX).
//!
//! The same payload carries a 24-hour `ai_decisions` activity summary —
//! identifiers, counts, and token totals only; never prompt, completion, or
//! mail content (dev/06 §9, 09 §5).

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::error::{AppResult, IpcError};
use crate::state::AppState;
use crate::storage::{map_sqlx_err, Db};
use crate::types::AiProvider;
use crate::util::now_unix;

/// Default OpenAI endpoint disclosed when the account has no `ai_base_url`
/// override (dev/06 §1; the adapter appends the `/v1` API path).
pub const OPENAI_DEFAULT_ENDPOINT: &str = "https://api.openai.com/v1";
/// Default Anthropic endpoint disclosed when no `ai_base_url` override is set.
pub const ANTHROPIC_DEFAULT_ENDPOINT: &str = "https://api.anthropic.com/v1";
/// Default Ollama endpoint (a local daemon on this device).
pub const OLLAMA_DEFAULT_ENDPOINT: &str = "http://localhost:11434";

/// The 24h activity-summary window (card §3: "last 24h").
const SUMMARY_WINDOW_SECS: i64 = 86_400;

// ── Wire DTOs (specta-exported via export.rs) ────────────────────────────────

/// Where one account's AI requests terminate (the disclosure classification,
/// dev/06 §8): a cloud endpoint, a localhost daemon, an in-process model, or
/// nowhere because AI is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AiEndpointKind {
    /// Mail content leaves the device for this endpoint when AI runs.
    Cloud,
    /// A daemon on this device (e.g. Ollama on localhost) — content stays local.
    Local,
    /// The model runs inside the SeekerMail process — no endpoint at all.
    InProcess,
    /// AI is disabled for the account — nothing is sent anywhere.
    None,
}

/// One per-account AI routing row for the disclosure panel (card §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiRouteEntry {
    pub account_id: String,
    pub account_email: String,
    /// `accounts.color_token` — drives the account badge in the panel.
    pub color_token: String,
    pub ai_provider: AiProvider,
    pub ai_model: Option<String>,
    pub endpoint_kind: AiEndpointKind,
    /// The full effective endpoint URL (`ai_base_url` override or the
    /// provider default). `None` for in-process and disabled rows.
    pub endpoint_url: Option<String>,
    /// Display form — authority only (`api.openai.com`, `localhost:11434`),
    /// no scheme or path (card §6: avoid over-technical URLs).
    pub endpoint_host: Option<String>,
    /// `true` for providers that never send mail content off this device
    /// (`ollama`, `local_onnx`). Cloud rows carry the disclosure note.
    pub is_local: bool,
}

/// One aggregated `ai_decisions` bucket for the 24h summary — counts and token
/// totals only, never content (09 §5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiActivityRow {
    pub account_id: String,
    /// `ai_decisions.decision_type` (capability slug / decision tag).
    pub decision_type: String,
    /// Model slug, or the `downgraded:*` placeholder for degraded calls.
    pub ai_model: Option<String>,
    pub request_count: u32,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

/// `get_data_flow_ai_routing` payload: routing rows + the 24h activity summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DataFlowAiRouting {
    pub routes: Vec<AiRouteEntry>,
    pub activity: Vec<AiActivityRow>,
    /// Unix seconds of the summary window start (`now - 24h`).
    pub since_unix: i64,
}

// ── Classification ───────────────────────────────────────────────────────────

/// Resolve the disclosure classification for one provider + optional
/// `ai_base_url` override (card §3 endpoint rules; ADR-0004: the panel shows
/// the *actual* endpoint the adapter will call).
fn classify_endpoint(
    provider: AiProvider,
    base_url: Option<&str>,
) -> (AiEndpointKind, Option<String>) {
    let effective = |default: &str| {
        base_url
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .unwrap_or(default)
            .to_string()
    };
    match provider {
        AiProvider::Openai => (
            AiEndpointKind::Cloud,
            Some(effective(OPENAI_DEFAULT_ENDPOINT)),
        ),
        AiProvider::Anthropic => (
            AiEndpointKind::Cloud,
            Some(effective(ANTHROPIC_DEFAULT_ENDPOINT)),
        ),
        AiProvider::Ollama => (
            AiEndpointKind::Local,
            Some(effective(OLLAMA_DEFAULT_ENDPOINT)),
        ),
        AiProvider::LocalOnnx => (AiEndpointKind::InProcess, None),
        AiProvider::None => (AiEndpointKind::None, None),
    }
}

/// `true` when the provider never sends mail content off this device
/// (mirrors the card's `is_local = ollama || local_onnx`).
fn is_local_provider(provider: AiProvider) -> bool {
    matches!(provider, AiProvider::Ollama | AiProvider::LocalOnnx)
}

/// The authority part of an endpoint URL for display: `https://api.openai.com/v1`
/// → `api.openai.com`; `http://localhost:11434` → `localhost:11434`.
fn endpoint_host(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let host = rest.split('/').next().unwrap_or(rest).trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

// ── Data access (Db-scoped, so tests run on an in-memory database) ──────────

/// Build the full disclosure payload at `now`. Accounts without an
/// `account_ai_settings` row classify as AI-off, same as `ai_provider = 'none'`.
pub async fn data_flow_ai_routing(db: &Db, now: i64) -> AppResult<DataFlowAiRouting> {
    /// `(id, email, color_token, ai_provider, ai_model, ai_base_url)`.
    type RouteRow = (
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let rows: Vec<RouteRow> = sqlx::query_as(
        "SELECT a.id, a.email, a.color_token, s.ai_provider, s.ai_model, s.ai_base_url \
         FROM accounts a \
         LEFT JOIN account_ai_settings s ON s.account_id = a.id \
         ORDER BY a.created_at, a.email",
    )
    .fetch_all(db.pool())
    .await
    .map_err(map_sqlx_err)?;

    let routes = rows
        .into_iter()
        .map(|(id, email, color_token, provider, model, base_url)| {
            let provider = provider
                .as_deref()
                .map(AiProvider::parse)
                .unwrap_or(AiProvider::None);
            let (endpoint_kind, endpoint_url) = classify_endpoint(provider, base_url.as_deref());
            let endpoint_host = endpoint_url.as_deref().and_then(endpoint_host);
            AiRouteEntry {
                account_id: id,
                account_email: email,
                color_token,
                ai_provider: provider,
                ai_model: model,
                endpoint_kind,
                endpoint_url,
                endpoint_host,
                is_local: is_local_provider(provider),
            }
        })
        .collect();

    let since_unix = now - SUMMARY_WINDOW_SECS;
    let summary: Vec<(String, String, Option<String>, i64, i64, i64)> = sqlx::query_as(
        "SELECT account_id, decision_type, ai_model, COUNT(*), \
                COALESCE(SUM(COALESCE(input_tokens, 0)), 0), \
                COALESCE(SUM(COALESCE(output_tokens, 0)), 0) \
         FROM ai_decisions WHERE created_at >= ? \
         GROUP BY account_id, decision_type, ai_model \
         ORDER BY account_id, decision_type, ai_model",
    )
    .bind(since_unix)
    .fetch_all(db.pool())
    .await
    .map_err(map_sqlx_err)?;

    let activity = summary
        .into_iter()
        .map(
            |(account_id, decision_type, ai_model, count, input, output)| AiActivityRow {
                account_id,
                decision_type,
                ai_model,
                request_count: count.max(0) as u32,
                input_tokens: input,
                output_tokens: output,
            },
        )
        .collect();

    Ok(DataFlowAiRouting {
        routes,
        activity,
        since_unix,
    })
}

// ── Command (registered in lib.rs `generate_handler!`) ──────────────────────

/// Per-account effective AI routing + the 24h `ai_decisions` summary for the
/// data-flow disclosure panel (T069, dev/06 §8).
#[tauri::command]
pub async fn get_data_flow_ai_routing(
    state: State<'_, AppState>,
) -> Result<DataFlowAiRouting, IpcError> {
    data_flow_ai_routing(state.storage.db(), now_unix())
        .await
        .map_err(IpcError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::new_uuid;

    async fn db() -> Db {
        let db = Db::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    /// Insert one account; `settings` controls whether an `account_ai_settings`
    /// row is written and with which provider/base_url.
    async fn seed_account(
        db: &Db,
        email: &str,
        created_at: i64,
        settings: Option<(&str, Option<&str>, Option<&str>)>,
    ) -> String {
        let id = new_uuid();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, created_at, updated_at) \
             VALUES (?, ?, 'Test', 'imap', 'slate', 'W', ?, ?)",
        )
        .bind(&id)
        .bind(email)
        .bind(created_at)
        .bind(created_at)
        .execute(db.pool())
        .await
        .unwrap();
        if let Some((provider, model, base_url)) = settings {
            sqlx::query(
                "INSERT INTO account_ai_settings (account_id, auth_level, ai_provider, ai_model, ai_base_url, daily_query_limit, updated_at) \
                 VALUES (?, 1, ?, ?, ?, 100, ?)",
            )
            .bind(&id)
            .bind(provider)
            .bind(model)
            .bind(base_url)
            .bind(created_at)
            .execute(db.pool())
            .await
            .unwrap();
        }
        id
    }

    /// Append one `ai_decisions` row (counts/tokens only, like the real writer).
    async fn seed_decision(
        db: &Db,
        account_id: &str,
        decision_type: &str,
        model: &str,
        input: i64,
        output: i64,
        created_at: i64,
    ) {
        sqlx::query(
            "INSERT INTO ai_decisions (id, account_id, decision_type, impact, action_description, \
                 result_description, ai_model, input_tokens, output_tokens, latency_ms, created_at) \
             VALUES (?, ?, ?, 'reply', 'unit-test row', 'completed', ?, ?, ?, 5, ?)",
        )
        .bind(new_uuid())
        .bind(account_id)
        .bind(decision_type)
        .bind(model)
        .bind(input)
        .bind(output)
        .bind(created_at)
        .execute(db.pool())
        .await
        .unwrap();
    }

    const NOW: i64 = 1_780_000_000;

    #[tokio::test]
    async fn openai_default_classifies_as_cloud_with_default_endpoint() {
        let db = db().await;
        seed_account(
            &db,
            "legal@example.com",
            1,
            Some(("openai", Some("gpt-4o"), None)),
        )
        .await;

        let payload = data_flow_ai_routing(&db, NOW).await.unwrap();
        assert_eq!(payload.routes.len(), 1);
        let row = &payload.routes[0];
        assert_eq!(row.ai_provider, AiProvider::Openai);
        assert_eq!(row.endpoint_kind, AiEndpointKind::Cloud);
        assert_eq!(row.endpoint_url.as_deref(), Some(OPENAI_DEFAULT_ENDPOINT));
        assert_eq!(row.endpoint_host.as_deref(), Some("api.openai.com"));
        assert_eq!(row.ai_model.as_deref(), Some("gpt-4o"));
        assert!(!row.is_local);
    }

    #[tokio::test]
    async fn base_url_override_is_disclosed_verbatim() {
        let db = db().await;
        // A Gemini-style vendor rides the openai variant with a custom base URL
        // (dev/06 §1) — the panel must show the real endpoint, not the default.
        seed_account(
            &db,
            "work@example.com",
            1,
            Some((
                "openai",
                None,
                Some("https://generativelanguage.googleapis.com/v1beta/openai"),
            )),
        )
        .await;

        let payload = data_flow_ai_routing(&db, NOW).await.unwrap();
        let row = &payload.routes[0];
        assert_eq!(row.endpoint_kind, AiEndpointKind::Cloud);
        assert_eq!(
            row.endpoint_url.as_deref(),
            Some("https://generativelanguage.googleapis.com/v1beta/openai")
        );
        assert_eq!(
            row.endpoint_host.as_deref(),
            Some("generativelanguage.googleapis.com")
        );
    }

    #[tokio::test]
    async fn anthropic_classifies_as_cloud() {
        let db = db().await;
        seed_account(&db, "a@example.com", 1, Some(("anthropic", None, None))).await;

        let payload = data_flow_ai_routing(&db, NOW).await.unwrap();
        let row = &payload.routes[0];
        assert_eq!(row.endpoint_kind, AiEndpointKind::Cloud);
        assert_eq!(
            row.endpoint_url.as_deref(),
            Some(ANTHROPIC_DEFAULT_ENDPOINT)
        );
        assert_eq!(row.endpoint_host.as_deref(), Some("api.anthropic.com"));
        assert!(!row.is_local);
    }

    #[tokio::test]
    async fn ollama_without_base_url_is_local_on_localhost() {
        let db = db().await;
        seed_account(
            &db,
            "p@example.com",
            1,
            Some(("ollama", Some("llama3:8b"), None)),
        )
        .await;

        let payload = data_flow_ai_routing(&db, NOW).await.unwrap();
        let row = &payload.routes[0];
        assert_eq!(row.endpoint_kind, AiEndpointKind::Local);
        assert_eq!(row.endpoint_url.as_deref(), Some(OLLAMA_DEFAULT_ENDPOINT));
        assert_eq!(row.endpoint_host.as_deref(), Some("localhost:11434"));
        assert!(row.is_local);
    }

    #[tokio::test]
    async fn local_onnx_is_in_process_with_no_endpoint() {
        let db = db().await;
        seed_account(&db, "x@example.com", 1, Some(("local_onnx", None, None))).await;

        let payload = data_flow_ai_routing(&db, NOW).await.unwrap();
        let row = &payload.routes[0];
        assert_eq!(row.endpoint_kind, AiEndpointKind::InProcess);
        assert_eq!(row.endpoint_url, None);
        assert_eq!(row.endpoint_host, None);
        assert!(row.is_local);
    }

    #[tokio::test]
    async fn none_and_missing_settings_rows_classify_as_ai_off() {
        let db = db().await;
        seed_account(&db, "off@example.com", 1, Some(("none", None, None))).await;
        // No account_ai_settings row at all — must degrade to AI-off, not error.
        seed_account(&db, "fresh@example.com", 2, None).await;

        let payload = data_flow_ai_routing(&db, NOW).await.unwrap();
        assert_eq!(payload.routes.len(), 2);
        for row in &payload.routes {
            assert_eq!(row.ai_provider, AiProvider::None);
            assert_eq!(row.endpoint_kind, AiEndpointKind::None);
            assert_eq!(row.endpoint_url, None);
            assert!(!row.is_local);
        }
    }

    #[tokio::test]
    async fn routes_are_ordered_and_one_per_account() {
        let db = db().await;
        seed_account(&db, "first@example.com", 1, Some(("openai", None, None))).await;
        seed_account(&db, "second@example.com", 2, Some(("ollama", None, None))).await;

        let payload = data_flow_ai_routing(&db, NOW).await.unwrap();
        let emails: Vec<&str> = payload
            .routes
            .iter()
            .map(|r| r.account_email.as_str())
            .collect();
        assert_eq!(emails, vec!["first@example.com", "second@example.com"]);
    }

    #[tokio::test]
    async fn summary_counts_and_token_totals_within_24h() {
        let db = db().await;
        let account = seed_account(&db, "a@example.com", 1, Some(("openai", None, None))).await;
        seed_decision(
            &db,
            &account,
            "draft_reply",
            "openai/gpt-4o",
            100,
            40,
            NOW - 600,
        )
        .await;
        seed_decision(
            &db,
            &account,
            "draft_reply",
            "openai/gpt-4o",
            200,
            60,
            NOW - 1_200,
        )
        .await;
        seed_decision(
            &db,
            &account,
            "summarize",
            "openai/gpt-4o",
            50,
            10,
            NOW - 300,
        )
        .await;

        let payload = data_flow_ai_routing(&db, NOW).await.unwrap();
        assert_eq!(payload.since_unix, NOW - 86_400);
        assert_eq!(payload.activity.len(), 2);

        let draft = payload
            .activity
            .iter()
            .find(|r| r.decision_type == "draft_reply")
            .unwrap();
        assert_eq!(draft.request_count, 2);
        assert_eq!(draft.input_tokens, 300);
        assert_eq!(draft.output_tokens, 100);
        assert_eq!(draft.ai_model.as_deref(), Some("openai/gpt-4o"));

        let summarize = payload
            .activity
            .iter()
            .find(|r| r.decision_type == "summarize")
            .unwrap();
        assert_eq!(summarize.request_count, 1);
        assert_eq!(summarize.input_tokens, 50);
    }

    #[tokio::test]
    async fn summary_excludes_rows_older_than_24h() {
        let db = db().await;
        let account = seed_account(&db, "a@example.com", 1, Some(("openai", None, None))).await;
        // Exactly on the boundary counts; one second older does not.
        seed_decision(&db, &account, "draft_reply", "m", 10, 5, NOW - 86_400).await;
        seed_decision(&db, &account, "draft_reply", "m", 999, 999, NOW - 86_401).await;

        let payload = data_flow_ai_routing(&db, NOW).await.unwrap();
        assert_eq!(payload.activity.len(), 1);
        assert_eq!(payload.activity[0].request_count, 1);
        assert_eq!(payload.activity[0].input_tokens, 10);
    }

    #[tokio::test]
    async fn summary_handles_null_tokens_from_degraded_rows() {
        let db = db().await;
        let account = seed_account(&db, "a@example.com", 1, Some(("openai", None, None))).await;
        // Degraded audit rows carry NULL token counts (fallback.rs) — the
        // aggregation must treat them as 0, not fail or skip the row.
        sqlx::query(
            "INSERT INTO ai_decisions (id, account_id, decision_type, impact, action_description, \
                 result_description, ai_model, created_at) \
             VALUES (?, ?, 'draft_reply', 'reply', 'unit-test row', 'degraded: x', 'downgraded:hold', ?)",
        )
        .bind(new_uuid())
        .bind(&account)
        .bind(NOW - 60)
        .execute(db.pool())
        .await
        .unwrap();

        let payload = data_flow_ai_routing(&db, NOW).await.unwrap();
        assert_eq!(payload.activity.len(), 1);
        assert_eq!(payload.activity[0].request_count, 1);
        assert_eq!(payload.activity[0].input_tokens, 0);
        assert_eq!(payload.activity[0].output_tokens, 0);
    }

    #[test]
    fn endpoint_host_strips_scheme_and_path() {
        assert_eq!(
            endpoint_host("https://api.openai.com/v1").as_deref(),
            Some("api.openai.com")
        );
        assert_eq!(
            endpoint_host("http://localhost:11434").as_deref(),
            Some("localhost:11434")
        );
        assert_eq!(
            endpoint_host("http://127.0.0.1:11434/").as_deref(),
            Some("127.0.0.1:11434")
        );
        assert_eq!(endpoint_host(""), None);
    }
}
