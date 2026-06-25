//! Typed Tauri event emitter (T024, 03 §13).
//!
//! Background tasks (scheduler, backfill, parse worker, attachment downloader)
//! push progress to the frontend through this one handle instead of touching
//! `tauri::AppHandle` directly. It is `Clone` (the handle is cheap to clone) and
//! has a `noop()` form for tests, where no webview exists.

pub mod types;

use serde::Serialize;

use crate::types::{AttachmentIndexProgressPayload, ExtractionProgressPayload};
use crate::types::{
    AttachmentProgressPayload, AttachmentReadyPayload, AutoLoopDetectedPayload, AutoSentPayload,
    DraftDiscardedPayload, DraftReadyPayload, DraftUpdatedPayload, ErrorCode,
    ExportCompletePayload, ExportErrorPayload, ExportProgressPayload, GteErrorPayload,
    GteFinishedPayload, GteProgressPayload, MailSummary, MailUpdatedPayload, PipelineErrorPayload,
    PrefsInvalidatedPayload, QueryExpiredPayload, QueryNewPayload, RiskAlertPayload,
    RiskResolvedPayload, SyncCompletePayload, SyncErrorPayload, SyncProgressPayload,
    SyncStartedPayload, WipeCompletePayload, WipeProgressPayload,
};
use types::name;

/// Broadcasts typed events to all webviews (currently one).
#[derive(Clone)]
pub struct Emitter {
    app: Option<tauri::AppHandle>,
}

impl Emitter {
    /// Wire the emitter to the running app.
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app: Some(app) }
    }

    /// A no-op emitter for tests / headless contexts.
    pub fn noop() -> Self {
        Self { app: None }
    }

    /// Internal: serialize + broadcast. A failed emit is logged, never fatal.
    fn emit<P: Serialize + Clone>(&self, event: &str, payload: P) {
        if let Some(app) = &self.app {
            use tauri::Emitter as _;
            if let Err(e) = app.emit(event, payload) {
                tracing::warn!(event_name = event, error = %e, "event emit failed");
            }
        }
    }

    // ── sync:* ──────────────────────────────────────────────────────────────

    pub fn sync_started(&self, account_id: &str) {
        self.emit(
            name::SYNC_STARTED,
            SyncStartedPayload {
                account_id: account_id.to_string(),
            },
        );
    }

    pub fn sync_progress(&self, account_id: &str, fetched: u32, total: Option<u32>, paused: bool) {
        self.emit(
            name::SYNC_PROGRESS,
            SyncProgressPayload {
                account_id: account_id.to_string(),
                fetched,
                total,
                paused,
            },
        );
    }

    pub fn sync_complete(&self, account_id: &str, new_count: u32) {
        self.emit(
            name::SYNC_COMPLETE,
            SyncCompletePayload {
                account_id: account_id.to_string(),
                new_count,
            },
        );
    }

    pub fn sync_error(&self, account_id: &str, code: ErrorCode, message: &str) {
        self.emit(
            name::SYNC_ERROR,
            SyncErrorPayload {
                account_id: account_id.to_string(),
                code,
                message: message.to_string(),
            },
        );
    }

    // ── mail:* ──────────────────────────────────────────────────────────────

    pub fn mail_new(&self, summary: MailSummary) {
        self.emit(name::MAIL_NEW, summary);
    }

    pub fn mail_updated(&self, payload: MailUpdatedPayload) {
        self.emit(name::MAIL_UPDATED, payload);
    }

    // ── attachment:* ────────────────────────────────────────────────────────

    pub fn attachment_progress(&self, attachment_id: &str, pct: u8) {
        self.emit(
            name::ATTACHMENT_PROGRESS,
            AttachmentProgressPayload {
                attachment_id: attachment_id.to_string(),
                pct,
            },
        );
    }

    pub fn attachment_ready(&self, attachment_id: &str, local_path: &str) {
        self.emit(
            name::ATTACHMENT_READY,
            AttachmentReadyPayload {
                attachment_id: attachment_id.to_string(),
                local_path: local_path.to_string(),
            },
        );
    }

    // ── extraction:* / attachment_index:* (T108/T109) ────────────────────────

    /// Attachment text-extraction backfill heartbeat (T108).
    pub fn extraction_progress(
        &self,
        processed: u64,
        total: u64,
        indexed: u64,
        skipped: u64,
        errored: u64,
    ) {
        self.emit(
            name::EXTRACTION_PROGRESS,
            ExtractionProgressPayload {
                processed,
                total,
                indexed,
                skipped,
                errored,
            },
        );
    }

    /// Attachment vector-index build heartbeat (T109).
    pub fn attachment_index_progress(&self, indexed: u64, total: u64, elapsed_ms: u64) {
        self.emit(
            name::ATTACHMENT_INDEX_PROGRESS,
            AttachmentIndexProgressPayload {
                indexed,
                total,
                elapsed_ms,
            },
        );
    }

    // ── gte:* ───────────────────────────────────────────────────────────────

    pub fn gte_progress(&self, indexed: u64, total_pending: u64, rate_per_sec: f32) {
        self.emit(
            name::GTE_PROGRESS,
            GteProgressPayload {
                indexed,
                total_pending,
                rate_per_sec,
            },
        );
    }

    pub fn gte_finished(&self, total_indexed: u64, elapsed_ms: u64) {
        self.emit(
            name::GTE_FINISHED,
            GteFinishedPayload {
                total_indexed,
                elapsed_ms,
            },
        );
    }

    pub fn gte_error(&self, mail_id: &str, reason: &str) {
        self.emit(
            name::GTE_ERROR,
            GteErrorPayload {
                mail_id: mail_id.to_string(),
                reason: reason.to_string(),
            },
        );
    }

    // ── export:* (T052) ─────────────────────────────────────────────────────

    pub fn export_progress(&self, task_id: &str, count: u64, total: u64, stage: &str) {
        self.emit(
            name::EXPORT_PROGRESS,
            ExportProgressPayload {
                task_id: task_id.to_string(),
                count,
                total,
                stage: stage.to_string(),
            },
        );
    }

    pub fn export_complete(
        &self,
        task_id: &str,
        output_path: &str,
        output_dir: &str,
        mail_count: u64,
    ) {
        self.emit(
            name::EXPORT_COMPLETE,
            ExportCompletePayload {
                task_id: task_id.to_string(),
                output_path: output_path.to_string(),
                output_dir: output_dir.to_string(),
                mail_count,
            },
        );
    }

    pub fn export_error(&self, task_id: &str, code: ErrorCode, message: &str) {
        self.emit(
            name::EXPORT_ERROR,
            ExportErrorPayload {
                task_id: task_id.to_string(),
                code,
                message: message.to_string(),
            },
        );
    }

    // ── wipe:* (T053) ───────────────────────────────────────────────────────

    pub fn wipe_progress(&self, task_id: &str, deleted: u64, total: u64) {
        self.emit(
            name::WIPE_PROGRESS,
            WipeProgressPayload {
                task_id: task_id.to_string(),
                deleted,
                total,
            },
        );
    }

    pub fn wipe_complete(&self, task_id: &str, freed_bytes: u64) {
        self.emit(
            name::WIPE_COMPLETE,
            WipeCompletePayload {
                task_id: task_id.to_string(),
                freed_bytes,
            },
        );
    }

    // ── style:* (T075) ──────────────────────────────────────────────────────

    pub fn style_progress(&self, account_id: &str, stage: &str, pct: u8) {
        self.emit(
            name::STYLE_PROGRESS,
            crate::ai::style::StyleProgressPayload {
                account_id: account_id.to_string(),
                stage: stage.to_string(),
                pct,
            },
        );
    }

    pub fn style_done(&self, account_id: &str, sample_count: i64) {
        self.emit(
            name::STYLE_DONE,
            crate::ai::style::StyleDonePayload {
                account_id: account_id.to_string(),
                sample_count,
            },
        );
    }

    pub fn style_error(&self, account_id: &str, code: ErrorCode) {
        self.emit(
            name::STYLE_ERROR,
            crate::ai::style::StyleErrorPayload {
                account_id: account_id.to_string(),
                code,
            },
        );
    }

    // ── draft:* (T077) ──────────────────────────────────────────────────────

    /// A generated AI draft was persisted to `ai_drafts` and is ready for the
    /// UI (E1 compose window / E6 review queue). Identifiers only — the body
    /// is fetched over IPC, never broadcast.
    pub fn draft_ready(&self, draft_id: &str, mail_id: &str, trigger_mode: &str, account_id: &str) {
        self.emit(
            name::DRAFT_READY,
            DraftReadyPayload {
                draft_id: draft_id.to_string(),
                mail_id: mail_id.to_string(),
                trigger_mode: trigger_mode.to_string(),
                account_id: account_id.to_string(),
            },
        );
    }

    /// A draft body was edited via `update_draft_body` (T080). Identifier
    /// only — the body is fetched over IPC, never broadcast.
    pub fn draft_updated(&self, draft_id: &str) {
        self.emit(
            name::DRAFT_UPDATED,
            DraftUpdatedPayload {
                draft_id: draft_id.to_string(),
            },
        );
    }

    /// A draft left the E6 review queue (T080): user discard, expiry sweep,
    /// supersession, or approval (`reason: "sent"` — consumed, not lost).
    pub fn draft_discarded(&self, draft_id: &str, reason: Option<&str>) {
        self.emit(
            name::DRAFT_DISCARDED,
            DraftDiscardedPayload {
                draft_id: draft_id.to_string(),
                reason: reason.map(str::to_string),
            },
        );
    }

    // ── auto:* / pipeline:* / risk:* (T082/T084/T085) ───────────────────────

    /// An E3 auto-reply was delivered after its undo window (T085).
    /// Identifiers only — the toast body never carries mail content.
    pub fn auto_sent(&self, draft_id: &str, account_id: &str, message_id: &str) {
        self.emit(
            name::AUTO_SENT,
            AutoSentPayload {
                draft_id: draft_id.to_string(),
                account_id: account_id.to_string(),
                message_id: message_id.to_string(),
            },
        );
    }

    /// The E3 loop guard stopped a thread's auto-replies (T085, F_E3 §6).
    pub fn auto_loop_detected(&self, thread_id: &str, account_id: &str) {
        self.emit(
            name::AUTO_LOOP_DETECTED,
            AutoLoopDetectedPayload {
                thread_id: thread_id.to_string(),
                account_id: account_id.to_string(),
            },
        );
    }

    /// One background pipeline job failed (T082). `error_code` is the wire
    /// `ErrorCode` string — never message text or content.
    pub fn pipeline_error(&self, mail_id: &str, error_code: &str) {
        self.emit(
            name::PIPELINE_ERROR,
            PipelineErrorPayload {
                mail_id: mail_id.to_string(),
                error_code: error_code.to_string(),
            },
        );
    }

    /// A new risk event exists (E4 interception, T084) — the UI refetches its
    /// risk queries. Identifiers only.
    pub fn risk_alert(&self, risk_event_id: &str, mail_id: &str, account_id: &str) {
        self.emit(
            name::RISK_ALERT,
            RiskAlertPayload {
                risk_event_id: risk_event_id.to_string(),
                mail_id: mail_id.to_string(),
                account_id: account_id.to_string(),
            },
        );
    }

    /// A risk event was resolved/dismissed (WB-16) — every window clears it from
    /// its T4 banner. Identifier only.
    pub fn risk_resolved(&self, risk_event_id: &str) {
        self.emit(
            name::RISK_RESOLVED,
            RiskResolvedPayload {
                risk_event_id: risk_event_id.to_string(),
            },
        );
    }

    // ── workbench:* (WB-13/14, Model S cross-window prefs) ────────────────────

    /// A global appearance pref changed in one window; others re-read at/after
    /// `revision` (WB-13). Broadcast to every webview.
    pub fn prefs_invalidated(&self, revision: u64) {
        self.emit(
            name::WORKBENCH_PREFS_INVALIDATED,
            PrefsInvalidatedPayload { revision },
        );
    }

    // ── query:* (I3 proactive queries, T095/T097) ────────────────────────────

    /// A proactive query was raised (T095). `priority` is `"high"` (the UI may
    /// push a notification) or `"normal"` (badge only). Identifiers only.
    pub fn query_new(&self, query_id: &str, account_id: &str, priority: &str) {
        self.emit(
            name::QUERY_NEW,
            QueryNewPayload {
                query_id: query_id.to_string(),
                account_id: account_id.to_string(),
                priority: priority.to_string(),
            },
        );
    }

    /// A query expired or fired a T4 reminder (T097). Identifiers only.
    pub fn query_expired(&self, query_id: &str, account_id: &str, trigger_type: &str) {
        self.emit(
            name::QUERY_EXPIRED,
            QueryExpiredPayload {
                query_id: query_id.to_string(),
                account_id: account_id.to_string(),
                trigger_type: trigger_type.to_string(),
            },
        );
    }

    // ── ai:* (T067) ─────────────────────────────────────────────────────────

    /// Every configured provider is cooling or unreachable — the F5 router
    /// entered the global-offline state (F_F5 §4.4).
    pub fn ai_offline(&self, reason: &str) {
        self.emit(
            name::AI_OFFLINE,
            crate::ai::fallback::AiOfflinePayload {
                reason: reason.to_string(),
            },
        );
    }

    /// A cooled provider passed its recovery probe; held tasks are being
    /// replayed with bounded catch-up (F_F5 §4.4).
    pub fn ai_online(&self, recovered_provider: &str) {
        self.emit(
            name::AI_ONLINE,
            crate::ai::fallback::AiOnlinePayload {
                recovered_provider: recovered_provider.to_string(),
            },
        );
    }
}

impl std::fmt::Debug for Emitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Emitter {{ wired: {} }}", self.app.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_emitter_is_silent() {
        // Must not panic without an app handle.
        let e = Emitter::noop();
        e.sync_started("acc");
        e.sync_progress("acc", 1, Some(10), false);
        e.mail_updated(MailUpdatedPayload {
            id: "m".into(),
            is_read: Some(true),
            is_starred: None,
        });
    }
}
