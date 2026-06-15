//! `AuditLogger` — the unified append-only writer for `ai_decisions` (T088).
//!
//! Every E-mode pipeline records its actions through this service:
//!
//! * [`AuditLogger::log`] is fire-and-forget (`tokio::spawn`); the pipeline is
//!   never blocked and a failed write is a `warn`, not an error.
//! * [`AuditLogger::log_await`] is the awaitable form for callers that need
//!   the row to exist before proceeding (e.g. command handlers under test).
//! * Callers already inside a transaction use
//!   [`super::repo::insert_decision_tx`] directly — the same single INSERT
//!   statement, so there is exactly one write path in the codebase.
//!
//! Log safety (09 §5): descriptions are short English summaries truncated to
//! [`DESCRIPTION_MAX_CHARS`]; callers must never put mail bodies, draft
//! bodies, or addresses in them, and this module never logs their values.

use crate::error::AppResult;
use crate::storage::Db;

/// Hard cap applied to `action_description` / `result_description` on write.
pub const DESCRIPTION_MAX_CHARS: usize = 200;

/// One audit record, pre-insert. Field semantics mirror `ai_decisions`
/// (dev/01); the row id and `created_at` are assigned at insert time.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub account_id: String,
    pub mail_id: Option<String>,
    pub draft_id: Option<String>,
    /// One of [`super::types::decision_type`].
    pub decision_type: String,
    /// `risk` | `reply` | `identity` | `rule` | `context` (dev/01).
    pub impact: String,
    /// English, ≤ 200 chars after truncation; never body/address text.
    pub action_description: String,
    /// English, ≤ 200 chars after truncation; never body/address text.
    pub result_description: String,
    pub knowledge_refs: Vec<String>,
    pub knowledge_summary: Option<String>,
    pub ai_model: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub latency_ms: Option<i64>,
}

/// Append-only `ai_decisions` writer. Cheap to clone (the pool is an `Arc`);
/// lives in `AppState` so commands and pipelines share one handle.
#[derive(Clone)]
pub struct AuditLogger {
    db: Db,
}

impl AuditLogger {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// Fire-and-forget write: spawns the INSERT so the calling pipeline never
    /// waits on the audit log (< 2 ms budget, F_E7 §7). A failed write is
    /// `warn`-logged with identifiers only — never the entry's text fields.
    pub fn log(&self, entry: AuditEntry) {
        let db = self.db.clone();
        tokio::spawn(async move {
            if let Err(e) = super::repo::insert_decision(&db, &entry).await {
                tracing::warn!(
                    event = "audit_log_write_failed",
                    code = e.code().as_wire(),
                    decision_type = %entry.decision_type,
                    account_id = %entry.account_id,
                    "audit log write failed"
                );
            }
        });
    }

    /// Awaitable write for callers that need confirmation the row landed.
    pub async fn log_await(&self, entry: AuditEntry) -> AppResult<()> {
        super::repo::insert_decision(&self.db, &entry)
            .await
            .map(|_| ())
    }
}

impl std::fmt::Debug for AuditLogger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AuditLogger")
    }
}
