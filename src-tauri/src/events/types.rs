//! Event names + typed dispatch enums (T024, 02 §4).
//!
//! The wire payload structs themselves live in `crate::types` (so specta exports
//! them); this module owns the stable event-name strings and small enums that
//! group related emissions for ergonomic dispatch.

/// Stable Tauri event names. Frontend `events.ts` listens on these exact strings.
pub mod name {
    pub const SYNC_STARTED: &str = "sync:started";
    pub const SYNC_PROGRESS: &str = "sync:progress";
    pub const SYNC_COMPLETE: &str = "sync:complete";
    pub const SYNC_ERROR: &str = "sync:error";
    pub const MAIL_NEW: &str = "mail:new";
    pub const MAIL_UPDATED: &str = "mail:updated";
    pub const ATTACHMENT_PROGRESS: &str = "attachment:progress";
    pub const ATTACHMENT_READY: &str = "attachment:ready";
    pub const EXTRACTION_PROGRESS: &str = "extraction:progress";
    pub const ATTACHMENT_INDEX_PROGRESS: &str = "attachment_index:progress";
    pub const GTE_PROGRESS: &str = "gte:progress";
    pub const GTE_FINISHED: &str = "gte:finished";
    pub const GTE_ERROR: &str = "gte:error";
    pub const EXPORT_PROGRESS: &str = "export:progress";
    pub const EXPORT_COMPLETE: &str = "export:complete";
    pub const EXPORT_ERROR: &str = "export:error";
    pub const WIPE_PROGRESS: &str = "wipe:progress";
    pub const WIPE_COMPLETE: &str = "wipe:complete";
    pub const STYLE_PROGRESS: &str = "style:progress";
    pub const STYLE_DONE: &str = "style:done";
    pub const STYLE_ERROR: &str = "style:error";
    pub const AI_OFFLINE: &str = "ai:offline";
    pub const AI_ONLINE: &str = "ai:online";
    pub const DRAFT_READY: &str = "draft:ready";
    pub const DRAFT_UPDATED: &str = "draft:updated";
    pub const DRAFT_DISCARDED: &str = "draft:discarded";
    pub const AUTO_SENT: &str = "auto:sent";
    pub const AUTO_LOOP_DETECTED: &str = "auto:loop_detected";
    pub const PIPELINE_ERROR: &str = "pipeline:error";
    pub const RISK_ALERT: &str = "risk:alert";
    /// A risk event was resolved/dismissed in one window (WB-16) — every window
    /// clears it from its T4 banner.
    pub const RISK_RESOLVED: &str = "risk:resolved";
    /// A global appearance pref changed in one window (WB-13/14) — others re-read.
    pub const WORKBENCH_PREFS_INVALIDATED: &str = "workbench:prefs_invalidated";
    pub const QUERY_NEW: &str = "query:new";
    pub const QUERY_EXPIRED: &str = "query:expired";
}

use crate::types::{
    AttachmentProgressPayload, AttachmentReadyPayload, GteErrorPayload, GteFinishedPayload,
    GteProgressPayload, MailSummary, MailUpdatedPayload, SyncCompletePayload, SyncErrorPayload,
    SyncProgressPayload, SyncStartedPayload,
};

/// `sync:*` family.
#[derive(Debug, Clone)]
pub enum SyncEvent {
    Started(SyncStartedPayload),
    Progress(SyncProgressPayload),
    Complete(SyncCompletePayload),
    Error(SyncErrorPayload),
}

/// `mail:*` family.
#[derive(Debug, Clone)]
pub enum MailEvent {
    New(MailSummary),
    Updated(MailUpdatedPayload),
}

/// `attachment:*` family.
#[derive(Debug, Clone)]
pub enum AttachEvent {
    Progress(AttachmentProgressPayload),
    Ready(AttachmentReadyPayload),
}

/// `gte:*` family — B3 embedding pipeline progress (T031).
#[derive(Debug, Clone)]
pub enum GteEvent {
    Progress(GteProgressPayload),
    Finished(GteFinishedPayload),
    Error(GteErrorPayload),
}

use crate::ai::style::{StyleDonePayload, StyleErrorPayload, StyleProgressPayload};

/// `style:*` family — E5 style learning (T075).
#[derive(Debug, Clone)]
pub enum StyleEvent {
    Progress(StyleProgressPayload),
    Done(StyleDonePayload),
    Error(StyleErrorPayload),
}

use crate::ai::fallback::{AiOfflinePayload, AiOnlinePayload};

/// `ai:*` family — F5 global AI availability (T067).
#[derive(Debug, Clone)]
pub enum AiStatusEvent {
    Offline(AiOfflinePayload),
    Online(AiOnlinePayload),
}

use crate::types::{DraftDiscardedPayload, DraftReadyPayload, DraftUpdatedPayload};

/// `draft:*` family — Module E draft generation + the E6 queue lifecycle
/// (T077/T080).
#[derive(Debug, Clone)]
pub enum DraftEvent {
    Ready(DraftReadyPayload),
    Updated(DraftUpdatedPayload),
    Discarded(DraftDiscardedPayload),
}

use crate::types::{
    AutoLoopDetectedPayload, AutoSentPayload, PipelineErrorPayload, RiskAlertPayload,
};

/// `auto:*` / `pipeline:*` / `risk:*` family — the E2/E3/E4 background
/// pipelines (T082/T084/T085).
#[derive(Debug, Clone)]
pub enum PipelineEvent {
    AutoSent(AutoSentPayload),
    LoopDetected(AutoLoopDetectedPayload),
    Error(PipelineErrorPayload),
    RiskAlert(RiskAlertPayload),
}
