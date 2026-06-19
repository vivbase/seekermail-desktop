//! Audit-log wire DTOs + decision-type vocabulary (T088, F_E7 §4).
//!
//! Everything here is structured metadata: identifiers, counts, token figures,
//! timestamps, and short English descriptions. Mail bodies, draft bodies, and
//! sender addresses never enter these shapes (09 §5).

use serde::{Deserialize, Serialize};
use specta::Type;

/// The `ai_decisions.decision_type` vocabulary (F_E7 §4.1). Stored as plain
/// strings so historical rows survive enum churn; these constants are the one
/// place the spellings live.
pub mod decision_type {
    pub const DRAFT_CREATED: &str = "draft_created";
    pub const DRAFT_SENT: &str = "draft_sent";
    pub const DRAFT_EDITED: &str = "draft_edited";
    pub const DRAFT_DISCARDED: &str = "draft_discarded";
    pub const DRAFT_REGENERATED: &str = "draft_regenerated";
    pub const AUTO_REPLY_SENT: &str = "auto_reply_sent";
    pub const AUTO_SEND_CANCELLED: &str = "auto_send_cancelled";
    pub const SENSITIVE_INTERCEPTED: &str = "sensitive_intercepted";
    pub const SPAM_TRASHED: &str = "spam_trashed";
    pub const DOWNGRADE_E3_TO_E2: &str = "downgrade_e3_to_e2";
    pub const PROVIDER_OFFLINE_FALLBACK: &str = "provider_offline_fallback";
    pub const GENERATION_FAILED: &str = "generation_failed";
    pub const NEEDS_REPLY_CHECK: &str = "needs_reply_check";
    pub const STYLE_UPDATED: &str = "style_updated";
    /// An agent answered a message in the shared TEAM channel (F_I5).
    pub const TEAM_REPLY: &str = "team_reply";
}

/// One `ai_decisions` row as the UI consumes it (F_E7 §4.4): the full audit
/// record plus the trigger mail's subject (LEFT JOIN — subject only, never the
/// body) so the list can label each event.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiDecisionRow {
    pub id: String,
    pub account_id: String,
    pub mail_id: Option<String>,
    pub draft_id: Option<String>,
    pub decision_type: String,
    /// `risk` | `reply` | `identity` | `rule` | `context` (dev/01).
    pub impact: String,
    pub action_description: String,
    pub result_description: String,
    pub knowledge_refs: Vec<String>,
    pub knowledge_summary: Option<String>,
    pub ai_model: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub latency_ms: Option<i64>,
    pub created_at: i64,
    /// Subject of the trigger mail, when it still exists (UI label only).
    pub mail_subject: Option<String>,
}

/// Filters for `list_ai_decisions` (F_E7 §4.5). All fields optional; `limit`
/// defaults to and is capped at 200, newest first.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListDecisionsParams {
    pub account_id: Option<String>,
    pub since_unix: Option<i64>,
    pub until_unix: Option<i64>,
    /// Restrict to these `decision_type` values (empty/None = all).
    pub decision_types: Option<Vec<String>>,
    pub impact: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Aggregated statistics over a time window (F_E7 §4.6) — computed in one
/// SQLite GROUP-BY pass, never by loading rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DecisionSummary {
    pub total_events: i64,
    pub auto_sent_count: i64,
    pub downgrade_count: i64,
    pub sensitive_count: i64,
    pub draft_sent_count: i64,
    pub draft_created_count: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    /// `draft_sent / draft_created`; `0.0` when nothing was created.
    pub success_rate: f64,
}

/// Output format for `export_ai_decisions` (F_E7 §4.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum AiDecisionExportFormat {
    Csv,
    Json,
}

impl AiDecisionExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            AiDecisionExportFormat::Csv => "csv",
            AiDecisionExportFormat::Json => "json",
        }
    }
}

/// Params for `export_ai_decisions` (F_E7 §4.7). The export carries structured
/// fields only — `action_description`, `result_description`, and
/// `knowledge_summary` stay local (privacy boundary, T088 §6).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportAiDecisionsParams {
    pub account_id: Option<String>,
    pub since_unix: i64,
    pub until_unix: i64,
    pub format: AiDecisionExportFormat,
}
