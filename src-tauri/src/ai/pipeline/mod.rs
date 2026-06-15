//! Background AI pipelines for Module E (T082 E2 semi-auto, T084 E4 pre-scan,
//! T085 E3 full-auto, T083 draft notifier).
//!
//! Flow per freshly ingested inbound mail (the parse worker enqueues an
//! [`worker::E2PipelineJob`]):
//!
//! ```text
//! ingest → worker → auth route ──Semi──▶ e4 → needs_reply → generate → draft:ready
//!                              └─Full──▶ e4 → gate/whitelist/loop/rate →
//!                                        needs_reply → generate → 6-point check →
//!                                        30 s send queue → auto:sent
//! ```
//!
//! Every module here follows the repo-wide conventions: runtime sqlx queries,
//! `map_sqlx_err`, identifiers-only logging (09 §5 — never subjects, bodies,
//! or addresses), and audit writes routed exclusively through
//! `ai::audit::repo` / `AuditLogger`.

pub mod e2_pipeline;
pub mod e3_checker;
pub mod e3_gate;
pub mod e3_pipeline;
pub mod e3_rate_limiter;
pub mod e3_send_queue;
pub mod e4_classifier;
pub mod e4_router;
pub mod i3_stage;
pub mod needs_reply;
pub mod notifier;
pub mod query_expiry;
pub mod resume;
pub mod worker;

use crate::error::AppResult;
use crate::storage::{map_sqlx_err, Db};

/// Global E2/E3 generation concurrency cap (F_E2 §4.6).
pub const PIPELINE_GLOBAL_CONCURRENCY: usize = 4;
/// Per-account generation concurrency cap (F_E2 §4.6).
pub const PIPELINE_PER_ACCOUNT_CONCURRENCY: usize = 2;

/// The mail-row snapshot the pipeline stages share. Loaded once per job so the
/// classifier, needs-reply checker, and E3 self-check all read the same state.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PipelineMail {
    pub id: String,
    pub account_id: String,
    pub thread_id: Option<String>,
    pub subject: String,
    pub from_email: String,
    /// JSON: `[{"name":"","email":""}]`.
    pub to_addrs: String,
    /// JSON: `[{"name":"","email":""}]`.
    pub cc_addrs: String,
    pub body_text: Option<String>,
    pub snippet: Option<String>,
    /// JSON array of IMAP flag strings (e.g. `"\\Junk"`).
    pub imap_flags: String,
    pub spam_score: Option<f64>,
    pub has_attachments: i64,
    pub is_sent: i64,
}

impl PipelineMail {
    /// Body text, falling back to the snippet when the body is absent.
    pub fn text(&self) -> &str {
        self.body_text
            .as_deref()
            .or(self.snippet.as_deref())
            .unwrap_or("")
    }

    /// Whether `email` appears in one of the stored recipient JSON arrays.
    fn addrs_contain(raw_json: &str, email: &str) -> bool {
        let needle = email.trim().to_lowercase();
        serde_json::from_str::<Vec<serde_json::Value>>(raw_json)
            .unwrap_or_default()
            .iter()
            .filter_map(|v| v.get("email").and_then(|e| e.as_str()))
            .any(|e| e.trim().to_lowercase() == needle)
    }

    pub fn to_contains(&self, email: &str) -> bool {
        Self::addrs_contain(&self.to_addrs, email)
    }

    pub fn cc_contains(&self, email: &str) -> bool {
        Self::addrs_contain(&self.cc_addrs, email)
    }

    /// Number of CC recipients on the original mail.
    pub fn cc_count(&self) -> usize {
        serde_json::from_str::<Vec<serde_json::Value>>(&self.cc_addrs)
            .map(|v| v.len())
            .unwrap_or(0)
    }
}

/// Load one non-deleted mail as a [`PipelineMail`]. `None` when the row is
/// missing or soft-deleted.
pub async fn load_mail(db: &Db, mail_id: &str) -> AppResult<Option<PipelineMail>> {
    let row: Option<PipelineMail> = sqlx::query_as(
        "SELECT id, account_id, thread_id, subject, from_email, to_addrs, cc_addrs, \
             body_text, snippet, imap_flags, spam_score, has_attachments, is_sent \
         FROM mails WHERE id = ? AND is_deleted = 0",
    )
    .bind(mail_id)
    .fetch_optional(db.pool())
    .await
    .map_err(map_sqlx_err)?;
    Ok(row)
}

/// The account's own address (needed by the TO/CC rule and loop detection).
pub async fn account_email(db: &Db, account_id: &str) -> AppResult<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT email FROM accounts WHERE id = ?")
        .bind(account_id)
        .fetch_optional(db.pool())
        .await
        .map_err(map_sqlx_err)?;
    Ok(row.map(|(e,)| e))
}
