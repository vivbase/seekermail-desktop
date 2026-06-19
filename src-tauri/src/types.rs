//! Shared DTOs — the single source of truth for cross-IPC types (T003).
//!
//! Every type the frontend consumes is defined here with `serde` + `specta::Type`
//! and exported to `packages/shared/src/bindings.ts` by `pnpm gen:types`
//! (the `gen-bindings` binary). The generated file is checked in and must never
//! be hand-edited; a CI drift check fails the build if it goes stale (02 §6, 08 §6).

use serde::{Deserialize, Serialize};
use specta::Type;

/// Result of the `ping` command — the first end-to-end IPC roundtrip (T002).
///
/// Started life as a bare `String` in T002 and was promoted to a named DTO in
/// T003 to establish the "every wire value is a named, generated type" pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PingReply {
    /// Always `"pong"` for a healthy backend.
    pub message: String,
}

/// The wire `ErrorCode` (02 §2). Serialized as `SCREAMING_SNAKE_CASE` strings so
/// the TypeScript union reads e.g. `"AUTH_INVALID_CREDENTIALS"`.
///
/// This is the authoritative set the frontend `errors.ts` table is keyed on; a
/// `Record<ErrorCode, …>` there forces a UX decision for every code at build time
/// (09 §4, 09 §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    // ── Auth / account ──────────────────────────────────────────────────────
    AuthInvalidCredentials,
    AuthOauthFailed,
    AuthKeychainDenied,
    // ── IMAP / SMTP ─────────────────────────────────────────────────────────
    ImapConnectionFailed,
    ImapUidValidityChanged,
    SmtpSendFailed,
    SmtpRateLimited,
    // ── Database ────────────────────────────────────────────────────────────
    DbNotFound,
    DbConstraint,
    DbMigrationFailed,
    // ── AI / GTE ────────────────────────────────────────────────────────────
    AiProviderUnreachable,
    AiRateLimited,
    AiContextTooLong,
    GteIndexCorrupt,
    GteReindexInProgress,
    // ── Filesystem ──────────────────────────────────────────────────────────
    FsPermissionDenied,
    FsDiskFull,
    // ── Generic ─────────────────────────────────────────────────────────────
    Validation,
    NotFound,
    Forbidden,
    Internal,
}

impl ErrorCode {
    /// The stable wire string (mirrors the serde rename). Used in logs and the
    /// `IpcError` payload.
    pub fn as_wire(&self) -> &'static str {
        match self {
            ErrorCode::AuthInvalidCredentials => "AUTH_INVALID_CREDENTIALS",
            ErrorCode::AuthOauthFailed => "AUTH_OAUTH_FAILED",
            ErrorCode::AuthKeychainDenied => "AUTH_KEYCHAIN_DENIED",
            ErrorCode::ImapConnectionFailed => "IMAP_CONNECTION_FAILED",
            ErrorCode::ImapUidValidityChanged => "IMAP_UID_VALIDITY_CHANGED",
            ErrorCode::SmtpSendFailed => "SMTP_SEND_FAILED",
            ErrorCode::SmtpRateLimited => "SMTP_RATE_LIMITED",
            ErrorCode::DbNotFound => "DB_NOT_FOUND",
            ErrorCode::DbConstraint => "DB_CONSTRAINT",
            ErrorCode::DbMigrationFailed => "DB_MIGRATION_FAILED",
            ErrorCode::AiProviderUnreachable => "AI_PROVIDER_UNREACHABLE",
            ErrorCode::AiRateLimited => "AI_RATE_LIMITED",
            ErrorCode::AiContextTooLong => "AI_CONTEXT_TOO_LONG",
            ErrorCode::GteIndexCorrupt => "GTE_INDEX_CORRUPT",
            ErrorCode::GteReindexInProgress => "GTE_REINDEX_IN_PROGRESS",
            ErrorCode::FsPermissionDenied => "FS_PERMISSION_DENIED",
            ErrorCode::FsDiskFull => "FS_DISK_FULL",
            ErrorCode::Validation => "VALIDATION",
            ErrorCode::NotFound => "NOT_FOUND",
            ErrorCode::Forbidden => "FORBIDDEN",
            ErrorCode::Internal => "INTERNAL",
        }
    }
}

// =============================================================================
// Module A — Accounts (T013–T018) and Module B — Mail processing (T023–T029).
//
// Every wire DTO below carries `#[serde(rename_all = "camelCase")]` so the
// generated TypeScript reads `displayName`, `imapOk`, `accountId`, … matching
// the frontend hooks (02 §1, T024 §6). Credentials are NEVER fields of any DTO
// returned to the frontend (01 `accounts` note, F_A2 §4).
// =============================================================================

/// Mail provider family. Drives OAuth endpoint selection (T015) and is persisted
/// in `accounts.provider` as a lowercase string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Gmail,
    Outlook,
    Imap,
    Exchange,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Gmail => "gmail",
            Provider::Outlook => "outlook",
            Provider::Imap => "imap",
            Provider::Exchange => "exchange",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "gmail" => Provider::Gmail,
            "outlook" => Provider::Outlook,
            "exchange" => Provider::Exchange,
            _ => Provider::Imap,
        }
    }

    /// Outlook/Microsoft uses a hosted OAuth authorization server. Gmail moved to
    /// IMAP + App Password (Google's `mail.google.com` is a restricted/paid scope —
    /// knowledge base `docs/analysis/29_*`); IMAP/Exchange use password auth.
    pub fn is_oauth(self) -> bool {
        matches!(self, Provider::Outlook)
    }
}

/// An email account / digital-employee identity (01 `accounts`). Credential-free
/// by construction — secrets live only in the Keychain (T006).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub provider: String,
    pub imap_host: Option<String>,
    pub imap_port: u16,
    pub smtp_host: Option<String>,
    pub smtp_port: u16,
    pub color_token: String,
    pub badge_label: String,
    pub role_type: String,
    pub role_description: Option<String>,
    pub auth_level: u8,
    pub is_primary: bool,
    pub is_active: bool,
    pub sync_interval_secs: u32,
    pub last_synced_at: Option<i64>,
    /// `None` = "all mail"; `Some(n)` = last `n` months (T016).
    pub knowledge_depth_months: Option<u32>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// The optional, cloud-backed SeekerMail ID (A6, decoupled model). It is
/// INDEPENDENT of any mailbox — created by signing in with Google, optional and
/// local-first. This is the local-cache projection returned to the UI; mail
/// content, contacts, and vectors never appear here. Spec: knowledge base
/// `docs/function list/F_A6_seekermail_id.md` + `docs/analysis/26_*`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SeekerMailId {
    /// Identity provider — `google` at launch; the schema is provider-agnostic.
    pub provider: String,
    /// The sign-in email (and the marketing-contact email when consented).
    pub email: String,
    pub display_name: Option<String>,
    pub email_verified: bool,
    /// Entitlement / subscription tier (transactional; not a marketing field).
    pub plan: Option<String>,
    /// Marketing email opt-in. Default OFF; first-party only (see PRIVACY_POLICY).
    pub marketing_consent: bool,
    pub marketing_consent_source: Option<String>,
    pub signed_in_at: i64,
}

/// Parameters to create an account (T013). `password` is consumed at the command
/// boundary and handed to the Keychain — it is never persisted in the DB and is
/// stripped from the returned [`Account`].
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateAccountParams {
    pub email: String,
    pub display_name: String,
    pub provider: Provider,
    pub imap_host: Option<String>,
    pub imap_port: Option<u16>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub color_token: String,
    pub badge_label: String,
    pub role_type: Option<String>,
    pub role_description: Option<String>,
    pub auth_level: Option<u8>,
    /// IMAP/SMTP password (omitted for OAuth accounts). Keychain-only.
    pub password: Option<String>,
}

/// Partial update for an account (T013). `email` is intentionally absent — it is
/// immutable once an account is created.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAccountParams {
    pub display_name: Option<String>,
    pub color_token: Option<String>,
    pub badge_label: Option<String>,
    pub role_type: Option<String>,
    pub role_description: Option<String>,
    pub auth_level: Option<u8>,
    pub is_active: Option<bool>,
    pub is_primary: Option<bool>,
    pub sync_interval_secs: Option<u32>,
    pub imap_host: Option<String>,
    pub imap_port: Option<u16>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
}

/// What `begin_oauth_flow` returns to the UI (T015): the authorize URL to open
/// plus the CSRF `state` nonce. The PKCE verifier stays server-side; tokens never
/// reach the frontend. The wizard passes `state` back to `complete_oauth_flow`
/// (deep-link callback or manual code paste).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OAuthBeginResult {
    pub authorize_url: String,
    pub state: String,
}

/// Built-in provider connection hints (T014 autodiscover).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHints {
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_tls: bool,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_tls: bool,
}

/// Input to the connection probe (T014). Plaintext `password` exists only inside
/// the probe's call frame; it is not stored anywhere.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VerifyConnectionParams {
    pub email: String,
    pub provider: Provider,
    pub password: Option<String>,
    pub imap_host: Option<String>,
    pub imap_port: Option<u16>,
    pub imap_tls: Option<bool>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_tls: Option<bool>,
}

/// In-band probe result (T014). The command returns `Ok(_)` even on probe
/// failure; only internal errors escalate to `IpcError` (09 §2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VerifyConnectionResult {
    pub imap_ok: bool,
    pub smtp_ok: bool,
    pub error_message: Option<String>,
}

/// One knowledge-depth time bucket's estimate (T016). `None` fields mean the
/// sample timed out / couldn't be computed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RangeEstimate {
    /// `None` = "all mail"; `Some(n)` = last `n` months.
    pub months: Option<u32>,
    pub mail_count: Option<u32>,
    pub estimated_mb: Option<u32>,
}

/// The six-bucket mailbox sample (T016).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SamplingResult {
    pub ranges: Vec<RangeEstimate>,
}

/// Per-account on-disk footprint (T020).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiskUsage {
    pub total_bytes: u64,
    pub attachment_bytes: u64,
    pub body_bytes: u64,
}

/// GTE / semantic-index engine statistics for the GTE + Repository pages.
/// Read-only aggregates over the local store (no live IMAP needed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GteStats {
    /// Total non-deleted mails.
    pub email_count: i64,
    /// Mails with a stored embedding (also the vector count).
    pub indexed_count: i64,
    /// Mails not yet represented in the vector index.
    pub unindexed_count: i64,
    /// Mails waiting in the embedding queue.
    pub queue_pending: i64,
    /// Mails skipped from indexing (spam / non-indexable).
    pub spam_excluded: i64,
    /// One vector per indexed mail.
    pub vector_count: i64,
    /// indexed / total, as a percentage (0–100).
    pub coverage_pct: f64,
    /// Embedding model name (e.g. `bge-m3`).
    pub model: String,
    /// Embedding dimensionality.
    pub dimensions: i64,
    /// Opaque index version label (from `app_settings`).
    pub index_version: String,
    /// Estimated vector-index footprint in bytes.
    pub storage_bytes: i64,
    /// Knowledge lookups recorded today (audit decisions citing knowledge).
    pub used_today: i64,
    /// Risk events raised today.
    pub risks_caught: i64,
    /// Active accounts (treated as syncing).
    pub accounts_syncing: i64,
    /// Most recent successful sync across accounts (unix seconds), if any.
    pub last_sync_at: Option<i64>,
}

/// One topic bucket with its decision count — the real analogue of the prototype
/// "Top Topics". Sourced from the AI decision log's `impact` classification
/// (risk / reply / identity / rule / context) since the deal-tag source was removed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TopicCount {
    /// Topic label (human-readable impact class).
    pub label: String,
    /// Design-system colour token for the bar.
    pub color: String,
    /// Number of AI decisions in this impact class.
    pub count: i64,
}

/// One indexed-mail "knowledge entry" for the GTE recent list + Repository browse
/// cards: the mail with its usage stats derived from `ai_decisions.knowledge_refs`
/// (how often / for what the agent cited it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeEntry {
    /// Mail id (also the vector row id).
    pub id: String,
    pub account_id: String,
    /// Owning account's design-system colour token.
    pub acct_color: String,
    /// Owning account's avatar badge label.
    pub acct_badge: String,
    pub subject: String,
    /// Short preview (mail snippet).
    pub excerpt: String,
    /// Body for the expanded card (sanitised HTML or plain text).
    pub body: String,
    /// Topic/category tags on the mail (empty since the deal-tag feature was removed).
    pub tags: Vec<String>,
    pub date_sent: i64,
    /// How many audit decisions cited this mail as knowledge.
    pub used_count: i64,
    /// Impact of the most recent citing decision (risk|reply|identity|rule|context).
    pub impact: String,
    /// Action of the most recent citing decision, if any.
    pub last_used_for: Option<String>,
    /// When it was most recently cited (unix seconds), if any.
    pub last_used_time: Option<i64>,
    /// Originating sender address.
    pub source: String,
    /// Thread subject (falls back to the mail subject).
    pub thread: String,
    /// When the mail was embedded into the index (unix seconds), if known.
    pub indexed_at: Option<i64>,
}

/// Per-account IMAP sync bookmark + health (01 `sync_state`, T021).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SyncState {
    pub account_id: String,
    pub last_sync_at: Option<i64>,
    pub last_sync_result: Option<String>,
    pub consecutive_errors: u32,
    pub backoff_until: Option<i64>,
    pub inbox_uid_validity: Option<i64>,
    pub inbox_uid_next: Option<i64>,
    pub full_sync_required: bool,
    pub total_mails_synced: u32,
    pub updated_at: i64,
}

/// History-backfill progress (T022, `backfill_state`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BackfillStatus {
    pub account_id: String,
    pub status: String,
    pub depth_months: Option<u32>,
    pub boundary_date: Option<i64>,
    pub last_uid_fetched: Option<i64>,
    pub total_uid_count: Option<u32>,
    pub fetched_count: u32,
    pub started_at: Option<i64>,
    pub paused_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub error_message: Option<String>,
    pub updated_at: i64,
}

/// Outcome of an `upsert_batch` (T023).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpsertStats {
    pub inserted: u32,
    pub skipped_duplicate: u32,
    pub parse_errors: u32,
}

/// Lightweight mail row for lists + the `mail:new` event payload (T023/T024).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MailSummary {
    pub id: String,
    pub account_id: String,
    pub thread_id: Option<String>,
    pub subject: String,
    pub from_name: Option<String>,
    pub from_email: String,
    pub snippet: Option<String>,
    pub date_sent: i64,
    pub is_read: bool,
    pub has_attachments: bool,
}

/// One thread row for the L0 list view (G2 `list_threads`). Mirrors the
/// `threads` table; `participants` is the decoded JSON email array.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
    pub account_id: String,
    pub subject: String,
    pub participants: Vec<String>,
    pub mail_count: u32,
    pub unread_count: u32,
    pub has_attachments: bool,
    pub latest_date: i64,
    pub snippet: Option<String>,
    pub is_archived: bool,
    pub is_starred: bool,
}

/// Full mail for the reading view (G3 `get_mail`). `to`/`cc` are decoded from the
/// stored JSON address arrays.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MailDetail {
    pub id: String,
    pub account_id: String,
    pub thread_id: Option<String>,
    pub subject: String,
    pub from_name: Option<String>,
    pub from_email: String,
    pub to: Vec<Recipient>,
    pub cc: Vec<Recipient>,
    pub date_sent: i64,
    pub body_html: Option<String>,
    pub body_text: Option<String>,
    pub is_read: bool,
    pub is_starred: bool,
    pub is_archived: bool,
    pub has_attachments: bool,
    pub folder: String,
}

/// Filter + pagination for `list_threads` (G2). Optional fields are absent =
/// "no filter"; `accountId = None` means all accounts.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListThreadsParams {
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub is_archived: Option<bool>,
    #[serde(default)]
    pub has_unread: Option<bool>,
    pub limit: i64,
    pub offset: i64,
}

/// Filter + pagination for `list_mails` (G3, flat list). `is_deleted` rows are
/// always excluded by the repo.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListMailsParams {
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub is_unread: Option<bool>,
    #[serde(default)]
    pub date_from: Option<i64>,
    #[serde(default)]
    pub date_to: Option<i64>,
    pub limit: i64,
    pub offset: i64,
}

/// Attachment metadata row for the reading view (T025/T026).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub mail_id: String,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub downloaded: bool,
    pub local_path: Option<String>,
    pub is_inline: bool,
    pub content_id: Option<String>,
    pub checksum_sha256: Option<String>,
}

/// Tracker/remote-image status for one mail (T029).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TrackerInfo {
    pub blocked: bool,
    pub tracker_count: u32,
    pub images_allowed: bool,
    pub sender_email: String,
}

/// Scope for a remote-image allow decision (T029). Tagged union → TS discriminated
/// union (`{ type: "thisMessage" } | { type: "alwaysSender", senderEmail }`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ImageAllowScope {
    ThisMessage,
    AlwaysSender {
        #[serde(rename = "senderEmail")]
        sender_email: String,
    },
}

// ── Event payloads (T024, 02 §4) — broadcast via the Emitter ────────────────

/// `sync:started`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SyncStartedPayload {
    pub account_id: String,
}

/// `sync:progress`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgressPayload {
    pub account_id: String,
    pub fetched: u32,
    pub total: Option<u32>,
    pub paused: bool,
}

/// `sync:complete`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SyncCompletePayload {
    pub account_id: String,
    pub new_count: u32,
}

/// `sync:error`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SyncErrorPayload {
    pub account_id: String,
    pub code: ErrorCode,
    pub message: String,
}

/// `mail:updated` — partial field update for one mail.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MailUpdatedPayload {
    pub id: String,
    pub is_read: Option<bool>,
    pub is_starred: Option<bool>,
}

/// `attachment:progress`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentProgressPayload {
    pub attachment_id: String,
    pub pct: u8,
}

/// `attachment:ready`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentReadyPayload {
    pub attachment_id: String,
    pub local_path: String,
}

// ── Attachment extraction + index (T108/T109, A5/C deepening · v0.6) ─────────

/// Returned by `start_attachment_extraction_backfill` (T108): how many downloaded
/// attachments are still awaiting text extraction.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionBatchStarted {
    pub pending_count: u32,
}

/// `extraction:progress` — attachment text-extraction heartbeat (T108).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionProgressPayload {
    pub processed: u64,
    pub total: u64,
    pub indexed: u64,
    pub skipped: u64,
    pub errored: u64,
}

/// Returned by `build_attachment_index` (T109): pending count + whether this call
/// started the build (`false` when one was already running).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentIndexBuildStatus {
    pub total_pending: u32,
    pub started: bool,
}

/// `attachment_index:progress` — attachment vector-index heartbeat (T109).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentIndexProgressPayload {
    pub indexed: u64,
    pub total: u64,
    pub elapsed_ms: u64,
}

// ── Attachment-hit search (T110, C3 extension · v0.6) ────────────────────────

/// Which search the combined command runs for the mail hits (T110).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    Keyword,
    Semantic,
    Auto,
}

/// Params for `search_with_attachments` (T110 §3a).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchWithAttachmentsParams {
    pub query: String,
    pub mode: SearchMode,
    pub account_id: Option<String>,
    pub date_from: Option<i64>,
    pub date_to: Option<i64>,
    pub limit: Option<i64>,
}

/// One attachment-origin search hit, enriched with its owning mail's metadata so
/// the search panel can render a full card without a second fetch (T110 §3a).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentHit {
    pub attachment_id: String,
    pub mail_id: String,
    pub filename: String,
    pub content_type: String,
    /// FTS5 `<mark>`-highlighted excerpt of the extracted text.
    pub excerpt: String,
    pub score: f32,
    pub mail_subject: String,
    pub mail_from_email: String,
    pub mail_date_sent: i64,
}

/// Combined result: mail-body hits + attachment-origin hits (T110 §3a).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchWithAttachmentsResult {
    pub mail_hits: Vec<SearchResult>,
    pub attachment_hits: Vec<AttachmentHit>,
}

/// `gte:progress` — embedding pipeline heartbeat (T031, B3).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GteProgressPayload {
    /// Mails indexed so far this worker session.
    pub indexed: u64,
    /// Mails still in `embedding_status='pending'`.
    pub total_pending: u64,
    /// Smoothed indexing rate (mails/second).
    pub rate_per_sec: f32,
}

/// `gte:finished` — emitted when the pending queue drains (T031).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GteFinishedPayload {
    pub total_indexed: u64,
    pub elapsed_ms: u64,
}

/// `gte:error` — a mail exhausted its embed retries (T031).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GteErrorPayload {
    pub mail_id: String,
    pub reason: String,
}

// =============================================================================
// Module C — Search (T032 keyword / T033 semantic / T035 saved searches)
// =============================================================================

/// Generic paginated envelope (02 §Pagination). `total` is the full match count
/// so the UI can render pagination without a second query.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PageResult<T> {
    pub items: Vec<T>,
    pub total: u32,
    pub offset: u32,
}

/// Relevance bucket for a result row (02 §SearchResult). high ≥ 0.7, mid ≥ 0.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ScoreLabel {
    High,
    Mid,
    Low,
}

impl ScoreLabel {
    /// Bucket a normalised 0–1 score (T032 §3, T033 §3).
    pub fn from_score(score: f32) -> Self {
        if score >= 0.7 {
            ScoreLabel::High
        } else if score >= 0.4 {
            ScoreLabel::Mid
        } else {
            ScoreLabel::Low
        }
    }
}

/// A single search hit, shared by keyword (C1) and semantic (C2) search.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub mail_id: String,
    pub account_id: String,
    pub subject: String,
    pub from_name: Option<String>,
    pub from_email: String,
    pub date_sent: i64,
    pub snippet: String,
    /// 0.0–1.0 — keyword: normalised BM25; semantic: cosine.
    pub score: f32,
    pub score_label: ScoreLabel,
    /// Matched fragments with `<mark>…</mark>` tags (keyword search only).
    pub highlights: Vec<String>,
}

/// Params for `keyword_search` (02 §Module C). `limit`/`offset` come from
/// `PageParams`; the backend clamps `limit` to `[1, 200]`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KeywordSearchParams {
    pub query: String,
    pub account_id: Option<String>,
    pub date_from: Option<i64>,
    pub date_to: Option<i64>,
    pub folder: Option<String>,
    pub limit: u32,
    pub offset: u32,
}

/// Params for `semantic_search` (02 §Module C).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSearchParams {
    pub query: String,
    pub account_id: Option<String>,
    /// Cross-account subset filter (T112). `None` = every active account;
    /// `Some(ids)` = only those accounts. `account_id`, when set, takes
    /// precedence (single-account path) for backward compatibility.
    pub account_filter: Option<Vec<String>>,
    pub date_from: Option<i64>,
    pub date_to: Option<i64>,
    /// Minimum cosine score; defaults to 0.35 when absent.
    pub min_score: Option<f32>,
    pub limit: u32,
    pub offset: u32,
}

/// A persisted saved search (01 `saved_searches`, T035).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SavedSearch {
    pub id: String,
    pub account_id: Option<String>,
    pub name: String,
    pub query: String,
    pub mode: String,
    pub sort_order: i32,
    pub created_at: i64,
}

/// Params to create a saved search (T035).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SaveSearchParams {
    pub name: String,
    pub query: String,
    pub mode: String,
    pub account_id: Option<String>,
}

/// One recent search (01 `search_history`, T032/T034).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchHistoryItem {
    pub id: i64,
    pub query: String,
    pub mode: String,
    pub result_count: Option<i64>,
    pub created_at: i64,
}

// =============================================================================
// Compose / send (T043) and drafts (T045)
// =============================================================================

/// One recipient (compose + send). `name` is optional; `email` is required.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Recipient {
    pub name: Option<String>,
    pub email: String,
}

/// Params for `send_mail` (02 §Module B/G). Recipients are typed addresses; the
/// HTML body is optional (plain-text-only messages are valid).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SendMailParams {
    pub account_id: String,
    pub to: Vec<Recipient>,
    #[serde(default)]
    pub cc: Vec<Recipient>,
    #[serde(default)]
    pub bcc: Vec<Recipient>,
    pub subject: String,
    pub body_text: String,
    pub body_html: Option<String>,
    /// Message-ID this is a reply to (drives thread association).
    pub in_reply_to: Option<String>,
    /// Space-separated References chain (RFC 2822).
    pub references: Option<String>,
    /// The draft this send finalises, if any (deleted on success, T045).
    pub draft_id: Option<String>,
}

/// `send_mail` result — the pending id (for cancel) + the message id (02 §3).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SendMailResult {
    pub pending_id: String,
    pub message_id: String,
}

/// `cancel_send` result (02 §3).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CancelSendResult {
    pub cancelled: bool,
}

/// A locally-stored compose draft (T045). Persisted to `ai_drafts` with
/// `trigger_mode='compose'` so user drafts and AI drafts share one table.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub id: String,
    pub account_id: String,
    pub to: Vec<Recipient>,
    pub cc: Vec<Recipient>,
    pub subject: String,
    pub body_text: String,
    pub body_html: Option<String>,
    pub in_reply_to: Option<String>,
    pub updated_at: i64,
}

/// Params to upsert a compose draft (T045). `id` absent → create.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SaveDraftParams {
    pub id: Option<String>,
    pub account_id: String,
    #[serde(default)]
    pub to: Vec<Recipient>,
    #[serde(default)]
    pub cc: Vec<Recipient>,
    pub subject: String,
    pub body_text: String,
    pub body_html: Option<String>,
    pub in_reply_to: Option<String>,
}

// ── Internal parse structs (T023) — NOT exported to the wire ────────────────
// These carry raw bytes (`ParsedAttachment::data`) and rich header fields used
// only inside the ingest pipeline, so they deliberately skip `specta::Type`.

/// A fully parsed inbound mail ready for `MailRepo::upsert_batch` (T023).
#[derive(Debug, Clone)]
pub struct ParsedMail {
    pub account_id: String,
    pub folder: String,
    pub imap_uid: Option<i64>,
    pub message_id: String,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub subject: String,
    pub from_name: Option<String>,
    pub from_email: String,
    pub to_addrs: String,
    pub cc_addrs: String,
    pub bcc_addrs: String,
    pub reply_to: Option<String>,
    pub date_sent: i64,
    pub date_received: i64,
    pub body_text: Option<String>,
    /// B1-sanitised HTML (already cleaned by the ingest worker, T027).
    pub body_html: Option<String>,
    pub snippet: Option<String>,
    pub has_attachments: bool,
    /// Tracker pixels detected by the sanitiser (T027); `> 0` sets `tracker_blocked`.
    pub tracker_count: u32,
    pub attachments: Vec<ParsedAttachment>,
}

/// One attachment extracted during MIME parse (T023). `data` is held only until
/// metadata is written; the bytes are streamed to disk by T025.
#[derive(Debug, Clone)]
pub struct ParsedAttachment {
    pub filename: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub content_id: Option<String>,
    pub is_inline: bool,
    /// 0-based ordinal position within the message's `attachments()` iterator.
    /// Persisted as `attachments.part_index` (migration 016) and used by the
    /// deferred byte-download (`ImapSession::fetch_part`) to re-address the exact
    /// MIME part on the server. Internal only — never crosses the IPC wire.
    pub part_index: u32,
    pub data: Option<Vec<u8>>,
}

/// Raw MIME envelope handed from the fetch tasks (T021/T022) to the parse worker
/// (T023) over an mpsc channel.
#[derive(Debug, Clone)]
pub struct RawMail {
    pub account_id: String,
    pub folder: String,
    pub imap_uid: i64,
    pub raw_bytes: Vec<u8>,
}

// ── BYO AI providers (T058, dev/06 §1, 02 §1) ───────────────────────────────

/// Which AI backend serves an account (02 §1 `AiProvider`). Persisted in
/// `account_ai_settings.ai_provider` as the lowercase wire string. `None` means
/// AI is disabled for the account (E0); Gemini-style vendors ride the `openai`
/// variant with a custom `ai_base_url` (dev/06 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AiProvider {
    Openai,
    Anthropic,
    Ollama,
    LocalOnnx,
    None,
}

impl AiProvider {
    /// Stable wire/db string (mirrors the serde representation).
    pub fn as_str(self) -> &'static str {
        match self {
            AiProvider::Openai => "openai",
            AiProvider::Anthropic => "anthropic",
            AiProvider::Ollama => "ollama",
            AiProvider::LocalOnnx => "local_onnx",
            AiProvider::None => "none",
        }
    }

    /// Parse the persisted db string; unknown values degrade to `None` (AI off)
    /// rather than erroring, so a downgraded build never blocks mail flow.
    pub fn parse(s: &str) -> Self {
        match s {
            "openai" => AiProvider::Openai,
            "anthropic" => AiProvider::Anthropic,
            "ollama" => AiProvider::Ollama,
            "local_onnx" => AiProvider::LocalOnnx,
            _ => AiProvider::None,
        }
    }

    /// Cloud providers require the F1/F3 data-flow disclosure before first use;
    /// local providers never send mail content off-device (dev/06 §1, §8).
    pub fn is_cloud(self) -> bool {
        matches!(self, AiProvider::Openai | AiProvider::Anthropic)
    }
}

// ── Settings & privacy policies (T050/T051) ─────────────────────────────────

/// Tracker-blocking policy, three levels (F_B2 §4.4). Persisted at
/// `app_settings.privacy.tracker_policy`; default `BlockKnown` (protection ON).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TrackerPolicy {
    BlockAll,
    BlockKnown,
    AllowAll,
}

impl TrackerPolicy {
    /// Stable wire/log tag (mirrors the serde representation).
    pub fn as_wire(&self) -> &'static str {
        match self {
            TrackerPolicy::BlockAll => "block_all",
            TrackerPolicy::BlockKnown => "block_known",
            TrackerPolicy::AllowAll => "allow_all",
        }
    }
}

/// Remote-image loading policy, three levels (F_B1). Persisted at
/// `app_settings.privacy.remote_image_policy`; default `BlockAll`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ImagePolicy {
    BlockAll,
    TrustedOnly,
    AllowAll,
}

impl ImagePolicy {
    /// Stable wire/log tag (mirrors the serde representation).
    pub fn as_wire(&self) -> &'static str {
        match self {
            ImagePolicy::BlockAll => "block_all",
            ImagePolicy::TrustedOnly => "trusted_only",
            ImagePolicy::AllowAll => "allow_all",
        }
    }
}

// ── Export (T052) ────────────────────────────────────────────────────────────

/// Export output format (F_H2 §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Mbox,
    Json,
}

impl ExportFormat {
    pub fn as_wire(&self) -> &'static str {
        match self {
            ExportFormat::Mbox => "mbox",
            ExportFormat::Json => "json",
        }
    }
}

/// Parameters for `start_export` (T052). Credentials are structurally absent —
/// the export pipeline reads only `mails`/`attachments` rows.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StartExportParams {
    pub account_ids: Vec<String>,
    /// Unix seconds, inclusive lower bound. `None` = no lower bound.
    pub date_from: Option<i64>,
    /// Unix seconds, inclusive upper bound. `None` = no upper bound.
    pub date_to: Option<i64>,
    pub format: ExportFormat,
    pub include_body: bool,
    pub include_attachments: bool,
}

/// `export:progress` — emitted once per processed batch.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgressPayload {
    pub task_id: String,
    pub count: u64,
    pub total: u64,
    /// "mails" | "attachments" | "zip"
    pub stage: String,
}

/// `export:complete`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportCompletePayload {
    pub task_id: String,
    /// Absolute path of the final `.zip` bundle.
    pub output_path: String,
    /// Absolute path of the directory holding the bundle.
    pub output_dir: String,
    pub mail_count: u64,
}

/// `export:error`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportErrorPayload {
    pub task_id: String,
    pub code: ErrorCode,
    pub message: String,
}

// ── Wipe / reindex / sync range (T053) ──────────────────────────────────────

/// What a wipe removes (F_H2 §4.2 two-tier + full removal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WipeScope {
    /// mails/threads/attachments rows + blobs; index untouched.
    MailsOnly,
    /// Same + LanceDB vectors and FTS entries.
    MailsAndIndex,
    /// Same + the account configuration row itself.
    Everything,
}

impl WipeScope {
    pub fn as_wire(&self) -> &'static str {
        match self {
            WipeScope::MailsOnly => "mails_only",
            WipeScope::MailsAndIndex => "mails_and_index",
            WipeScope::Everything => "everything",
        }
    }
}

/// Impact preview shown before the `DELETE` confirmation step.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WipePreview {
    pub mail_count: u64,
    pub attachment_count: u64,
    pub estimated_bytes: u64,
}

/// `wipe:progress` — emitted per deleted batch.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WipeProgressPayload {
    pub task_id: String,
    pub deleted: u64,
    pub total: u64,
}

/// `wipe:complete`. `freed_bytes` is the estimate confirmed after VACUUM.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WipeCompletePayload {
    pub task_id: String,
    pub freed_bytes: u64,
}

/// Preview for a sync-range shrink (how many local mails fall outside).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SyncRangePreview {
    pub mails_beyond_range: u64,
}

// =============================================================================
// Module F — BYO AI (T059, 02 §Module H, dev/06)
//
// Key material is structurally absent from every DTO returned to the frontend:
// `ai_api_key_ref` (the Keychain item reference) and the key itself never
// appear in `AccountAiSettings` (ADR-0004, dev/06 §0). The two param types
// that *carry* a key inbound redact it in their hand-written `Debug` impls so
// no derive can ever print it (09 §5).
// =============================================================================

/// Per-account AI configuration (`account_ai_settings`, 01) as seen by the
/// frontend (02 §1 `AccountAiSettings`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AccountAiSettings {
    pub account_id: String,
    /// 1 = Manual Only, 2 = Semi-Auto, 3 = Full Auto.
    pub auth_level: u8,
    pub ai_provider: AiProvider,
    pub ai_model: Option<String>,
    pub ai_base_url: Option<String>,
    pub t1_enabled: bool,
    pub t2_enabled: bool,
    pub t3_enabled: bool,
    pub t4_enabled: bool,
    pub t5_enabled: bool,
    pub t6_enabled: bool,
    pub daily_query_limit: u32,
    pub e3_whitelist_only: bool,
    pub e3_min_history: u32,
    pub style_samples_count: u32,
    pub updated_at: i64,
}

/// Partial update for `account_ai_settings` (02 §Module H). `ai_api_key` is
/// consumed at the command boundary and written to the Keychain — it is never
/// stored in a DB column and never echoed back to the frontend.
#[derive(Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAiSettingsParams {
    pub auth_level: Option<u8>,
    pub ai_provider: Option<AiProvider>,
    pub ai_model: Option<String>,
    /// Written to the Keychain at the boundary; `None` = leave unchanged.
    pub ai_api_key: Option<String>,
    pub ai_base_url: Option<String>,
    pub t1_enabled: Option<bool>,
    pub t2_enabled: Option<bool>,
    pub t3_enabled: Option<bool>,
    pub t4_enabled: Option<bool>,
    pub t5_enabled: Option<bool>,
    pub t6_enabled: Option<bool>,
    pub daily_query_limit: Option<u32>,
    pub e3_whitelist_only: Option<bool>,
    pub e3_min_history: Option<u32>,
}

impl std::fmt::Debug for UpdateAiSettingsParams {
    /// Hand-written so the inbound API key can never reach a log line or a
    /// panic message via a derived `Debug` (09 §5).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateAiSettingsParams")
            .field("auth_level", &self.auth_level)
            .field("ai_provider", &self.ai_provider)
            .field("ai_model", &self.ai_model)
            .field("ai_api_key", &self.ai_api_key.as_ref().map(|_| "***"))
            .field("ai_base_url", &self.ai_base_url)
            .field("t1_enabled", &self.t1_enabled)
            .field("t2_enabled", &self.t2_enabled)
            .field("t3_enabled", &self.t3_enabled)
            .field("t4_enabled", &self.t4_enabled)
            .field("t5_enabled", &self.t5_enabled)
            .field("t6_enabled", &self.t6_enabled)
            .field("daily_query_limit", &self.daily_query_limit)
            .field("e3_whitelist_only", &self.e3_whitelist_only)
            .field("e3_min_history", &self.e3_min_history)
            .finish()
    }
}

/// Input to `verify_ai_provider` (02 §Module H): test a provider key/endpoint
/// without saving. The transient key exists only inside the command frame.
#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VerifyAiProviderParams {
    pub provider: AiProvider,
    pub model: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
}

impl std::fmt::Debug for VerifyAiProviderParams {
    /// Hand-written so the transient API key can never reach a log line via a
    /// derived `Debug` (09 §5).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifyAiProviderParams")
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "***"))
            .field("base_url", &self.base_url)
            .finish()
    }
}

/// In-band result of `verify_ai_provider` (02 §Module H). The command returns
/// `Ok(_)` even when the probe fails (09 §2 in-band bucket); `error_message`
/// carries only the sanitized, content-free `ProviderError` rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VerifyAiProviderResult {
    pub ok: bool,
    /// Confirmed model name from the provider, when it reports one.
    pub model_name: Option<String>,
    pub error_message: Option<String>,
}

/// Input to `list_cloud_models` (T068): read a cloud provider's model catalog
/// (`GET /v1/models`) for the add-cloud-provider model picker, without saving.
/// The transient key exists only inside the command frame.
#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListCloudModelsParams {
    pub provider: AiProvider,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
}

impl std::fmt::Debug for ListCloudModelsParams {
    /// Hand-written so the transient API key can never reach a log line via a
    /// derived `Debug` (09 §5) — mirrors [`VerifyAiProviderParams`].
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListCloudModelsParams")
            .field("provider", &self.provider)
            .field("api_key", &self.api_key.as_ref().map(|_| "***"))
            .field("base_url", &self.base_url)
            .finish()
    }
}

// =============================================================================
// Module D — AI role analysis (T070 legal / F_D1 §4.4)
//
// Wire DTOs for `analyze_legal_risk`. `originalText` excerpts travel in this
// in-memory result; the persisted `risk_events.evidence` carries only a
// SHA-256 prefix of each excerpt, never the excerpt itself (T070 §6, 09 §5).
// =============================================================================

/// Severity of one identified risk item (D1 output schema, F_D1 §4.4). The
/// model must emit exactly one of these lowercase tags — anything else fails
/// validation and triggers the single re-prompt retry (T070 §3 step 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum LegalRiskLevel {
    High,
    Medium,
    Low,
}

/// Worst-of aggregate over `riskList` (T070 §3): `high` maps to a T4 alert,
/// `none` means the analysis found no risks at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum LegalOverallLevel {
    High,
    Medium,
    Low,
    None,
}

impl LegalOverallLevel {
    /// Stable wire/log tag (mirrors the serde representation). Used in the
    /// `ai_decisions.result_description` summary and boundary logs.
    pub fn as_wire(&self) -> &'static str {
        match self {
            LegalOverallLevel::High => "high",
            LegalOverallLevel::Medium => "medium",
            LegalOverallLevel::Low => "low",
            LegalOverallLevel::None => "none",
        }
    }
}

/// D1 risk category (F_D1 §4.4). Mapped to `risk_events.risk_type` by the
/// legal pipeline (T070 §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum LegalRiskType {
    Payment,
    Delivery,
    Liability,
    Confidentiality,
    Dispute,
    Other,
}

/// One identified risk (F_D1 §4.4). `originalText` is capped at 120 chars and
/// `finding`/`suggestion` at 80 chars — enforced defensively at parse time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LegalRiskItem {
    pub level: LegalRiskLevel,
    #[serde(rename = "type")]
    pub risk_type: LegalRiskType,
    pub original_text: String,
    pub finding: String,
    pub suggestion: String,
}

/// Standard key-clause extraction (F_D1 §4.4). Clauses the model did not find
/// are `None` and the UI renders them as absent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LegalKeyClauses {
    pub payment: Option<String>,
    pub delivery: Option<String>,
    pub liability: Option<String>,
    pub confidentiality: Option<String>,
    pub dispute_resolution: Option<String>,
}

/// Input to `analyze_legal_risk` (T070 §3).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeLegalRiskParams {
    pub mail_id: String,
    /// `false` (the default) returns the cached analysis when one exists
    /// within the last 24 hours — no provider call is made (F_D1 §4.5).
    #[serde(default)]
    pub force_new: bool,
}

/// The D1 legal analysis verdict returned to the frontend (T070 §3). Also the
/// exact JSON persisted in `ai_decisions.action_description`, which backs the
/// 24-hour result cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LegalAnalysisResult {
    /// `ai_decisions.id` of this analysis (the E7 audit row).
    pub decision_id: String,
    pub mail_id: String,
    pub account_id: String,
    pub risk_list: Vec<LegalRiskItem>,
    pub key_clauses: LegalKeyClauses,
    pub compliance_advice: Vec<String>,
    /// Derived: the worst `riskList[].level` (T070 §6).
    pub overall_level: LegalOverallLevel,
    pub ai_model: String,
    /// Mail ids of the GTE chunks that grounded the analysis (dev/06 §9).
    pub knowledge_refs: Vec<String>,
    pub created_at: i64,
}

// =============================================================================
// Module E — risk events (T071; dev/01 `risk_events`, dev/02 §Module E)
//
// Wire DTOs for `list_risk_events` / `resolve_risk_event`. Rows are written by
// the D1 legal analyzer (`ai::legal`) and the E4 router (`ai::pipeline::e4_router`);
// these two commands are the read + resolve surface that backs the T4 banner and
// the Report risk panel. Mirrors `src/ipc/legal.ts` (`RiskEvent`, params).
// =============================================================================

/// One `risk_events` row returned to the frontend (T071). `risk_level` is 1–6
/// (T1–T6); `expires_at` is `null` for T4 — those never expire
/// (AI_MODES_DESIGN §8.1, root CLAUDE.md T4 rule). `evidence` is the structured
/// JSON object persisted with the event; it never contains the flagged excerpt
/// itself, only a hash (T070 §6, dev/09 §5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RiskEvent {
    pub id: String,
    pub mail_id: String,
    pub account_id: String,
    pub risk_level: i64,
    pub risk_type: String,
    pub evidence: serde_json::Value,
    pub description: String,
    pub status: String,
    pub expires_at: Option<i64>,
    pub created_at: i64,
}

/// Filter for `list_risk_events` (T071). Every field is optional; `status`
/// defaults to `open` when omitted, so the banner and report panel see only
/// live risks (mirrors the off-Tauri mock contract). `mail_id` powers the
/// per-mail T4 banner.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListRiskEventsParams {
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub mail_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub risk_level: Option<i64>,
}

/// Input to `resolve_risk_event` (T071). `status` is the terminal state to set
/// (`resolved` or `dismissed`); `dismissed` is rejected for T4 events, which are
/// non-dismissable until resolved (root CLAUDE.md T4 rule).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ResolveRiskParams {
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub resolution_note: Option<String>,
}

// =============================================================================
// Module D — AI role analysis (T072 sales / F_D2 §4.4)
//
// Wire DTOs for `analyze_sales_context`. D2 is context assistance, not a
// safety risk: the pipeline writes one `ai_decisions` audit row and never
// touches `risk_events` (T072 §3 step 9).
// =============================================================================

/// Counterparty stance read from the mail (F_D2 §4.4). The model must emit
/// exactly one of these lowercase tags — anything else fails validation and
/// triggers the single re-prompt retry (T072 §3 step 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum CounterpartyStance {
    Cooperative,
    Neutral,
    Adversarial,
}

impl CounterpartyStance {
    /// Stable wire/log tag (mirrors the serde representation). Used in the
    /// `ai_decisions.result_description` summary and boundary logs.
    pub fn as_wire(&self) -> &'static str {
        match self {
            CounterpartyStance::Cooperative => "cooperative",
            CounterpartyStance::Neutral => "neutral",
            CounterpartyStance::Adversarial => "adversarial",
        }
    }
}

/// Counterparty communication style (F_D2 §4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum CounterpartyTone {
    Formal,
    Casual,
}

/// Priority of one identified need (F_D2 §4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum NeedPriority {
    High,
    Medium,
    Low,
}

/// Suggested execution window for one next action (F_D2 §4.4). Wire tags are
/// the D2 schema literals: `immediate` / `24h` / `72h` / `this_week`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum NextActionTimeline {
    #[serde(rename = "immediate")]
    Immediate,
    #[serde(rename = "24h")]
    Within24h,
    #[serde(rename = "72h")]
    Within72h,
    #[serde(rename = "this_week")]
    ThisWeek,
}

/// Counterparty profile block of the D2 verdict (F_D2 §4.4): stance, tone,
/// decision-authority clues, and free-form observations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CounterpartyProfile {
    pub stance: CounterpartyStance,
    pub tone: CounterpartyTone,
    pub authority_signal: String,
    pub observations: Vec<String>,
}

/// One explicit or between-the-lines need (F_D2 §4.4). `evidence` is an
/// original-text snippet capped at 80 chars — enforced defensively at parse
/// time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NeedItem {
    pub need: String,
    pub priority: NeedPriority,
    pub evidence: String,
}

/// Three-tier concession guidance (F_D2 §4.4): what we can concede, what is
/// negotiable, and what must not be conceded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConcessionAdvice {
    pub concedable: Vec<String>,
    pub negotiable: Vec<String>,
    pub non_concedable: Vec<String>,
}

/// One recommended next step with its execution window (F_D2 §4.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NextAction {
    pub action: String,
    pub timeline: NextActionTimeline,
}

/// Counterparty history snapshot from `contacts` (F_D2 §4.2). `None` on the
/// wire when this is the first contact — the analysis then rests on the
/// current mail alone (F_D2 §6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ContactHistorySummary {
    pub interaction_count: i64,
    pub reply_count: i64,
    /// Parsed `contacts.style_notes` JSON; `None` when absent or unreadable.
    pub style_notes: Option<serde_json::Value>,
}

/// Input to `analyze_sales_context` (T072 §3).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeSalesContextParams {
    pub mail_id: String,
    /// `false` (the default) returns the cached analysis when one exists
    /// within the last 24 hours — no provider call is made (T072 §3 step 1).
    #[serde(default)]
    pub force_new: bool,
}

/// The D2 sales analysis returned to the frontend (T072 §3). Also the exact
/// JSON persisted in `ai_decisions.action_description`, which backs the
/// 24-hour result cache.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SalesAnalysisResult {
    /// `ai_decisions.id` of this analysis (the E7 audit row).
    pub decision_id: String,
    pub mail_id: String,
    pub account_id: String,
    pub counterparty_profile: CounterpartyProfile,
    pub needs_and_intents: Vec<NeedItem>,
    pub concession_advice: ConcessionAdvice,
    pub next_actions: Vec<NextAction>,
    /// `None` when the counterparty has no `contacts` row (first contact).
    pub contact_history: Option<ContactHistorySummary>,
    pub ai_model: String,
    /// Mail ids of the GTE chunks that grounded the analysis (dev/06 §9).
    pub knowledge_refs: Vec<String>,
    pub created_at: i64,
}

// =============================================================================
// Module F — provider configuration UI (T068, 02 §Module H, F_F1 §5, F_F2 §3)
//
// Wire DTOs for `scan_local_providers`, `list_ollama_models`, and
// `list_configured_providers`. All three are read-only summaries: no field
// here ever carries key material — `ConfiguredProviderInfo` is projected from
// the same key-free columns as `AccountAiSettings` (ADR-0004, dev/06 §0).
// =============================================================================

/// One reachable local AI endpoint found by `scan_local_providers` (F_F2 §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LocalProviderEndpoint {
    /// Normalized base URL, e.g. `http://localhost:11434`.
    pub base_url: String,
    /// Daemon kind behind the endpoint; v0.5 probes Ollama defaults only.
    pub provider: AiProvider,
}

/// One model installed on a local daemon (F_F2 §4.3) — the wire shape of
/// `list_ollama_models`, projected from the adapter's `OllamaModelInfo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OllamaModelEntry {
    /// Full model tag, e.g. `llama3:8b`.
    pub name: String,
    /// On-disk weight size in bytes (0 when the daemon omits it).
    pub size_bytes: u64,
    /// e.g. `8B` — from the daemon's `details.parameter_size`.
    pub parameter_size: Option<String>,
    /// e.g. `Q4_0` — from the daemon's `details.quantization_level`.
    pub quantization: Option<String>,
}

/// Provider summary for one account whose AI is configured (T068 §3). One row
/// per account with `ai_provider != 'none'`; consumed by the Settings → AI
/// Providers list and the T066 matrix UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConfiguredProviderInfo {
    pub account_id: String,
    pub email: String,
    pub display_name: String,
    /// Account color token (`terra` / `slate` / `sage` / …) for the row accent.
    pub color_token: String,
    pub provider: AiProvider,
    pub model: Option<String>,
    pub base_url: Option<String>,
    /// 1 = Manual Only, 2 = Semi-Auto, 3 = Full Auto.
    pub auth_level: u8,
    /// Local providers never send mail content off-device (F_F2 §4.5 badge).
    pub is_local: bool,
    /// Whether an adapter or factory for this provider is registered in this
    /// build (`AiRegistry::registered`) — `false` renders as unavailable.
    pub available: bool,
    pub updated_at: i64,
}

// =============================================================================
// Module E — AI reply drafts (T077, 02 §Module E, dev/01 §ai_drafts)
//
// The wire mirror of one `ai_drafts` row plus the `request_ai_reply` /
// `regenerate_draft` param shapes and the `draft:ready` event payload. Bodies
// travel over IPC to the local webview only — they are never logged (09 §5).
// =============================================================================

/// One AI-generated reply draft (`ai_drafts`, dev/01). `body_original` is the
/// immutable AI output; `body_current` reflects user edits (E6).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiDraft {
    pub id: String,
    pub trigger_mail_id: String,
    pub account_id: String,
    /// Reply recipient — the trigger mail's sender.
    pub to_addr: Recipient,
    pub cc_addrs: Vec<Recipient>,
    pub subject: String,
    pub body_original: String,
    pub body_current: String,
    pub is_edited: bool,
    pub style_match_score: Option<f64>,
    /// `E1_manual` | `E2_semi` | `E3_auto`.
    pub trigger_mode: String,
    pub ai_model: String,
    /// Source mail ids of the GTE context used for generation (T074).
    pub knowledge_refs: Vec<String>,
    /// `pending` | `edited` | `sent` | `discarded` | `expired`.
    pub status: String,
    pub send_after: Option<i64>,
    pub expires_at: Option<i64>,
    pub sent_at: Option<i64>,
    pub discarded_at: Option<i64>,
    /// `user` | `expired` | `superseded`.
    pub discard_reason: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Params for `request_ai_reply` (02 §Module E): generate an E1 manual reply
/// draft for one mail, with an optional steering instruction.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RequestAiReplyParams {
    pub mail_id: String,
    pub instruction: Option<String>,
}

/// Params for `regenerate_draft` (02 §Module E): supersede an existing draft
/// and generate a fresh one for the same trigger mail.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RegenerateDraftParams {
    /// The draft being regenerated (it ends up `discarded`/`superseded`).
    pub id: String,
    pub instruction: Option<String>,
}

/// `draft:ready` payload (T077) — a generated draft landed in `ai_drafts`;
/// the UI opens the compose window / review queue from these identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DraftReadyPayload {
    pub draft_id: String,
    pub mail_id: String,
    /// `E1_manual` | `E2_semi` | `E3_auto`.
    pub trigger_mode: String,
    pub account_id: String,
}

/// Filter for `list_pending_drafts` (T080, 02 §Module E).
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ListAiDraftsParams {
    /// `null` = all accounts.
    pub account_id: Option<String>,
    /// Default 50, capped at 200.
    pub limit: Option<i64>,
}

/// Result of `approve_draft` (T090, E6 "Approve & Send"). `pendingId` feeds
/// the existing `cancel_send` command while the 10 s SMTP window is open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ApproveDraftResult {
    pub sent_at: i64,
    pub message_id: String,
    /// Present when the send went through the queued path (undo window).
    pub pending_id: Option<String>,
}

/// `draft:updated` payload (T080) — a draft body changed via
/// `update_draft_body`; identifiers only, the body travels over IPC on demand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DraftUpdatedPayload {
    pub draft_id: String,
}

/// `draft:discarded` payload (T080) — the draft left the review queue.
/// `reason`: `user` | `expired` | `superseded` | `sent` (consumed by approve).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DraftDiscardedPayload {
    pub draft_id: String,
    pub reason: Option<String>,
}

/// `auto:sent` payload (T085) — an E3 auto-reply left after its undo window.
/// Identifiers only; the frontend toast carries no mail content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AutoSentPayload {
    pub draft_id: String,
    pub account_id: String,
    pub message_id: String,
}

/// `auto:loop_detected` payload (T085) — the loop guard stopped a thread's
/// auto-replies (chain length reached the cap).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AutoLoopDetectedPayload {
    pub thread_id: String,
    pub account_id: String,
}

/// `pipeline:error` payload (T082) — one background pipeline job failed;
/// `error_code` is the wire `ErrorCode` string, never message text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PipelineErrorPayload {
    pub mail_id: String,
    pub error_code: String,
}

/// `risk:alert` payload (T084) — an E4 interception (or any new risk event)
/// the UI should refetch risk queries for. Identifiers only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RiskAlertPayload {
    pub risk_event_id: String,
    pub mail_id: String,
    pub account_id: String,
}

// =============================================================================
// Module I — Agent-IM / TEAM channel (T092, F_I2 §5)
//
// One row of the single shared group channel. `content` is a JSON string whose
// shape depends on `message_type` (plain `{ "text": "…" }` for text/status,
// the QA-card schema for query_card). Storing it as an opaque string keeps the
// wire contract stable while I3/I4 evolve the card schema (T098).
// =============================================================================

/// One Agent-IM channel message (`im_messages`, T092). The TEAM channel is a
/// single shared room — `channel_id` is always `"main"` (no private chats).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ImMessage {
    pub id: String,
    pub channel_id: String,
    /// `human` | `agent` | `system`.
    pub sender_type: String,
    /// Account id for `agent`; `"human"` / `"system"` for the other senders.
    pub sender_id: String,
    /// `text` | `query_card` | `card_reply` | `status`.
    pub message_type: String,
    /// JSON payload; shape depends on `message_type` (text → `{ "text": "…" }`).
    pub content: String,
    pub linked_email_id: Option<String>,
    /// `pending` | `answered` | `skipped` | `resolved` (query-card lifecycle).
    pub status: String,
    pub created_at: i64,
    pub read_at: Option<i64>,
}

/// Derived Agent presence for one account (T094, F_I2 §4.2). Not persisted — it is
/// computed from `sync_state` + recent `ai_drafts`: `offline` when the account's
/// last sync failed auth/network, `processing` when a draft was generated in the
/// last 5 minutes, otherwise `idle`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub account_id: String,
    /// `processing` | `idle` | `offline`.
    pub status: String,
}

/// One proactive query awaiting a human decision (`pending_queries`, T095/I3).
/// `content`-rich rendering lives in the `im_messages` query card (T098); this is
/// the lifecycle row the Pending DecisionCard (T099) and resume/expiry chains
/// (T096/T097) act on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PendingQuery {
    pub id: String,
    pub account_id: String,
    pub mail_id: Option<String>,
    pub risk_event_id: Option<String>,
    /// `T1`–`T6`.
    pub trigger_type: String,
    pub question: String,
    /// JSON array of option labels (the rich QA card content lives in im_messages).
    pub options: Option<String>,
    pub answer: Option<String>,
    /// `pending` | `answered` | `skipped` | `expired`.
    pub status: String,
    /// 1 (highest) – 5; T4 is always 1.
    pub priority: i64,
    /// `None` for T4 (never expires); otherwise `created_at + 72h`.
    pub expires_at: Option<i64>,
    pub answered_at: Option<i64>,
    pub created_at: i64,
}

/// `query:new` payload (T095/T101) — a proactive query was raised. Identifiers
/// only; `priority` is `"high"` (push a notification) or `"normal"` (badge only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QueryNewPayload {
    pub query_id: String,
    pub account_id: String,
    pub priority: String,
}

/// `query:expired` payload (T097/T101) — a query auto-expired (non-T4) or fired a
/// T4 reminder. Identifiers only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QueryExpiredPayload {
    pub query_id: String,
    pub account_id: String,
    pub trigger_type: String,
}
