//! Network transport seams (T014/T015/T018/T021/T022).
//!
//! Every outbound protocol — IMAP, SMTP, and the OAuth token HTTP endpoint — is
//! reached through a trait object so the service logic above it is exercised with
//! in-memory fakes (the cards' explicit "test seam": T014 §8, T015 §8, T021 §8).
//! Those fakes live in [`fakes`] (compiled under `#[cfg(test)]`) and are injected
//! into [`crate::state::AppState`] via `test_state_with_net`.
//!
//! * The DEFAULT build wires the `offline` implementations: they compile with no
//!   network crates and return a clean "offline" failure / empty result, so the
//!   app boots and the logic layers are fully unit-testable.
//! * `--features live-net` swaps in the concrete `live` adapters (async-imap /
//!   lettre / reqwest). Those are isolated here so a change in a pre-1.0 crate API
//!   never ripples into the service or command layers.
//!
//! Async is expressed with `futures::future::BoxFuture` rather than an
//! `async_trait` dependency, keeping the trait objects `dyn`-safe.

use std::sync::Arc;

use futures::future::BoxFuture;

use crate::error::AppResult;
// `AppError` is referenced only by the offline-only `offline_err` below; importing
// it unconditionally warns as unused under `--features live-net`.
#[cfg(not(feature = "live-net"))]
use crate::error::AppError;

#[cfg(test)]
pub mod fakes;
#[cfg(feature = "live-net")]
mod live;
#[cfg(not(feature = "live-net"))]
mod offline;

// ── Shared value types ──────────────────────────────────────────────────────

/// Plaintext IMAP credentials. Exists only inside a transport call frame.
#[derive(Debug, Clone)]
pub struct ImapCreds {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub email: String,
    pub secret: String,
}

/// Plaintext SMTP credentials.
#[derive(Debug, Clone)]
pub struct SmtpCreds {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub email: String,
    pub secret: String,
}

/// `SELECT INBOX` result (01 `sync_state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InboxStatus {
    pub uid_validity: i64,
    pub uid_next: i64,
    pub exists: u32,
}

/// Outcome of one IMAP IDLE wait (push sync). The listener treats *any* server
/// change as "go fetch" — the dedup cursor in `poll_once` decides what is new.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleOutcome {
    /// The server reported a mailbox change — fetch new mail now.
    MailArrived,
    /// The keepalive window elapsed with no change — re-issue IDLE.
    TimedOut,
}

/// In-band connection-probe report (T014). Mirrors [`crate::types::VerifyConnectionResult`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnProbeReport {
    pub imap_ok: bool,
    pub smtp_ok: bool,
    pub error_message: Option<String>,
}

/// Inputs to a connection probe (T014).
#[derive(Debug, Clone)]
pub struct ConnProbeConfig {
    pub imap: ImapCreds,
    pub smtp: SmtpCreds,
}

/// OAuth token-endpoint request (authorization-code exchange OR refresh).
#[derive(Debug, Clone)]
pub struct TokenRequest {
    pub token_url: String,
    pub client_id: String,
    pub redirect_uri: String,
    /// `Some(code)+Some(verifier)` for the initial exchange; `None` for refresh.
    pub code: Option<String>,
    pub code_verifier: Option<String>,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
}

/// OAuth token-endpoint response.
#[derive(Debug, Clone)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in_secs: i64,
}

// ── Transport traits ────────────────────────────────────────────────────────

/// An open, authenticated IMAP session over the INBOX (T021/T022).
pub trait ImapSession: Send {
    fn select_inbox(&mut self) -> BoxFuture<'_, AppResult<InboxStatus>>;
    /// UID list of messages with internal-date ≥ `since_epoch`, newest first.
    fn search_uids_since(&mut self, since_epoch: i64) -> BoxFuture<'_, AppResult<Vec<i64>>>;
    /// UID list of messages with `UID ≥ uid_from` (incremental poll).
    fn search_uids_from(&mut self, uid_from: i64) -> BoxFuture<'_, AppResult<Vec<i64>>>;
    /// `FETCH BODY.PEEK[]` raw RFC-822 bytes for each requested UID.
    #[allow(clippy::type_complexity)]
    fn fetch_bodies(&mut self, uids: &[i64]) -> BoxFuture<'_, AppResult<Vec<(i64, Vec<u8>)>>>;
    /// Raw bytes for a single attachment, addressed by its 0-based index within
    /// the message's `attachments()` iterator (the value stored in
    /// `attachments.part_index`, migration 016). The live adapter fetches the full
    /// message and slices out that part, which is correct for any MIME nesting.
    fn fetch_part(&mut self, uid: i64, part_index: u32) -> BoxFuture<'_, AppResult<Vec<u8>>>;
    /// Block in IMAP IDLE until the server reports a mailbox change or `max_wait`
    /// elapses (then a re-IDLE keepalive is due). Requires a prior `select_inbox`.
    /// Used only by the push listener ([`crate::imap::idle_task`]); the interval
    /// poll path never calls it.
    fn idle_wait(&mut self, max_wait: std::time::Duration)
        -> BoxFuture<'_, AppResult<IdleOutcome>>;
}

/// Opens [`ImapSession`]s. One per build flavour (offline / live).
pub trait ImapFactory: Send + Sync {
    fn open(&self, creds: ImapCreds) -> BoxFuture<'_, AppResult<Box<dyn ImapSession>>>;
}

/// Performs the IMAP + SMTP connection probe (T014). In-band: returns a report,
/// never an `Err`, unless something internal breaks.
pub trait ConnProbe: Send + Sync {
    fn verify(&self, cfg: ConnProbeConfig) -> BoxFuture<'_, ConnProbeReport>;
}

/// Talks to an OAuth provider's token endpoint (T015/T018).
pub trait TokenEndpoint: Send + Sync {
    fn exchange(&self, req: TokenRequest) -> BoxFuture<'_, AppResult<TokenResponse>>;
}

/// The bundle of transports carried by [`crate::state::AppState`]. Cloneable —
/// every field is an `Arc<dyn …>` so background tasks share one instance.
#[derive(Clone)]
pub struct Net {
    pub imap: Arc<dyn ImapFactory>,
    pub probe: Arc<dyn ConnProbe>,
    pub oauth: Arc<dyn TokenEndpoint>,
}

impl Net {
    /// Wire the transports appropriate to the active build (live-net vs offline).
    pub fn resolve() -> Self {
        #[cfg(feature = "live-net")]
        {
            Net {
                imap: Arc::new(live::LiveImapFactory::new()),
                probe: Arc::new(live::LiveConnProbe::new()),
                oauth: Arc::new(live::LiveTokenEndpoint::new()),
            }
        }
        #[cfg(not(feature = "live-net"))]
        {
            Net {
                imap: Arc::new(offline::OfflineImapFactory),
                probe: Arc::new(offline::OfflineConnProbe),
                oauth: Arc::new(offline::OfflineTokenEndpoint),
            }
        }
    }
}

impl std::fmt::Debug for Net {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Net { .. }")
    }
}

/// Helper for offline/live adapters: a uniform "this build can't reach the
/// network" error.
#[cfg(not(feature = "live-net"))]
pub(crate) fn offline_err(what: &str) -> AppError {
    AppError::ImapConnection(format!(
        "{what}: offline build (enable --features live-net)"
    ))
}
