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

/// `SELECT` result for a folder (01 `sync_state`). Named for the INBOX it was
/// first used with; [`ImapSession::select_folder`] returns the same shape for any
/// folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InboxStatus {
    pub uid_validity: i64,
    pub uid_next: i64,
    pub exists: u32,
}

/// The role of a server folder, resolved from RFC 6154 SPECIAL-USE attributes
/// (with a leaf-name fallback for servers that don't advertise them). Drives the
/// multi-folder fetch allow-list and the local `mails.folder` tag on ingest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderRole {
    Inbox,
    Sent,
    Junk,
    Trash,
    Drafts,
    Archive,
    All,
    Other,
}

impl FolderRole {
    /// Best-effort role from a mailbox name, for servers that don't advertise
    /// SPECIAL-USE. Case-insensitive; matches the last path segment so
    /// `[Gmail]/Sent Mail` and `INBOX.Sent` both resolve to `Sent`.
    pub fn from_name_heuristic(name: &str) -> FolderRole {
        if name.eq_ignore_ascii_case("INBOX") {
            return FolderRole::Inbox;
        }
        let leaf = name
            .rsplit(['/', '.'])
            .next()
            .unwrap_or(name)
            .trim()
            .to_ascii_lowercase();
        match leaf.as_str() {
            "sent" | "sent mail" | "sent items" | "sent messages" => FolderRole::Sent,
            "junk" | "spam" | "junk email" | "junk e-mail" | "bulk mail" => FolderRole::Junk,
            "trash" | "deleted" | "deleted items" | "deleted messages" | "bin" => FolderRole::Trash,
            "drafts" | "draft" => FolderRole::Drafts,
            "all mail" => FolderRole::All,
            "archive" | "archives" => FolderRole::Archive,
            _ => FolderRole::Other,
        }
    }

    /// The canonical local `mails.folder` tag for this role, or `None` for folders
    /// SeekerMail does not ingest in the read-side pass (Drafts/Archive/All/Other).
    /// The allow-list is therefore exactly INBOX / SENT / JUNK / TRASH.
    pub fn local_folder_tag(self) -> Option<&'static str> {
        match self {
            FolderRole::Inbox => Some("INBOX"),
            FolderRole::Sent => Some("SENT"),
            FolderRole::Junk => Some("JUNK"),
            FolderRole::Trash => Some("TRASH"),
            _ => None,
        }
    }

    /// The inverse of [`Self::local_folder_tag`]: the role behind a stored
    /// `mails.folder` tag. Lets the write-back worker map a tag back to the live
    /// server mailbox name (which differs, e.g. `[Gmail]/Sent Mail`).
    pub fn from_local_tag(tag: &str) -> Option<FolderRole> {
        match tag {
            "INBOX" => Some(FolderRole::Inbox),
            "SENT" => Some(FolderRole::Sent),
            "JUNK" => Some(FolderRole::Junk),
            "TRASH" => Some(FolderRole::Trash),
            _ => None,
        }
    }
}

/// One folder discovered by [`ImapSession::list_folders`]: the server-side
/// selectable name plus its resolved [`FolderRole`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailboxFolder {
    pub name: String,
    pub role: FolderRole,
}

/// A message's server-side system flags, read during inbound reconciliation
/// (server→local read/star state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MessageFlags {
    pub seen: bool,
    pub flagged: bool,
}

/// A message flag SeekerMail writes back to the server (RFC 3501 system flags).
/// The write-back side of two-way sync: a local read/star toggle becomes a
/// `UID STORE` of the matching flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImapFlag {
    /// `\Seen` — read / unread.
    Seen,
    /// `\Flagged` — starred.
    Flagged,
}

impl ImapFlag {
    /// The IMAP wire token (backslash-prefixed system flag).
    pub fn as_imap_token(self) -> &'static str {
        match self {
            ImapFlag::Seen => "\\Seen",
            ImapFlag::Flagged => "\\Flagged",
        }
    }
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
    /// List the server's selectable folders with their resolved SPECIAL-USE roles
    /// (RFC 6154). Lets the multi-folder scheduler find SENT / JUNK / TRASH
    /// without hard-coding provider-specific names (Gmail's `[Gmail]/Sent Mail`
    /// vs a plain `Sent`).
    fn list_folders(&mut self) -> BoxFuture<'_, AppResult<Vec<MailboxFolder>>>;
    /// `SELECT` an arbitrary folder by name (generalises [`Self::select_inbox`],
    /// which is kept for the INBOX-only IDLE and sampler paths).
    fn select_folder(&mut self, name: &str) -> BoxFuture<'_, AppResult<InboxStatus>>;
    /// `UID STORE` one system flag on a single message — the write-back primitive
    /// for read/starred state (two-way sync). `set` adds the flag (`+FLAGS`),
    /// `!set` removes it (`-FLAGS`). Selects `folder` first, so it is
    /// self-contained.
    fn store_flag(
        &mut self,
        folder: &str,
        uid: i64,
        flag: ImapFlag,
        set: bool,
    ) -> BoxFuture<'_, AppResult<()>>;
    /// `UID MOVE` a message from `source_folder` to `dest_folder` — the write-back
    /// primitive for archive / delete (→ Trash) / mark-spam (→ Junk). Selects the
    /// source first, so it is self-contained. Requires server MOVE support
    /// (RFC 6851; Gmail / Outlook / iCloud / Dovecot all have it).
    fn move_message(
        &mut self,
        source_folder: &str,
        uid: i64,
        dest_folder: &str,
    ) -> BoxFuture<'_, AppResult<()>>;
    /// `UID FETCH uid_from:* (FLAGS)` — the current system flags of every message
    /// at or above `uid_from` in `folder`. Drives inbound reconciliation: the
    /// caller updates local read/star state from the returned flags and treats any
    /// locally-held UID missing from the result as vanished (moved/deleted on the
    /// server). Selects `folder` first, so it is self-contained.
    fn fetch_flags(
        &mut self,
        folder: &str,
        uid_from: i64,
    ) -> BoxFuture<'_, AppResult<Vec<(i64, MessageFlags)>>>;
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

#[cfg(test)]
mod tests {
    use super::FolderRole;

    #[test]
    fn name_heuristic_resolves_common_folders() {
        assert_eq!(FolderRole::from_name_heuristic("INBOX"), FolderRole::Inbox);
        assert_eq!(FolderRole::from_name_heuristic("inbox"), FolderRole::Inbox);
        assert_eq!(
            FolderRole::from_name_heuristic("[Gmail]/Sent Mail"),
            FolderRole::Sent
        );
        assert_eq!(
            FolderRole::from_name_heuristic("INBOX.Sent"),
            FolderRole::Sent
        );
        assert_eq!(FolderRole::from_name_heuristic("Junk"), FolderRole::Junk);
        assert_eq!(
            FolderRole::from_name_heuristic("[Gmail]/Spam"),
            FolderRole::Junk
        );
        assert_eq!(
            FolderRole::from_name_heuristic("Deleted Items"),
            FolderRole::Trash
        );
        assert_eq!(
            FolderRole::from_name_heuristic("[Gmail]/All Mail"),
            FolderRole::All
        );
        assert_eq!(
            FolderRole::from_name_heuristic("Work/Clients"),
            FolderRole::Other
        );
    }

    #[test]
    fn local_folder_tag_is_the_ingest_allow_list() {
        assert_eq!(FolderRole::Inbox.local_folder_tag(), Some("INBOX"));
        assert_eq!(FolderRole::Sent.local_folder_tag(), Some("SENT"));
        assert_eq!(FolderRole::Junk.local_folder_tag(), Some("JUNK"));
        assert_eq!(FolderRole::Trash.local_folder_tag(), Some("TRASH"));
        // Not ingested in the read-side pass.
        assert_eq!(FolderRole::Drafts.local_folder_tag(), None);
        assert_eq!(FolderRole::Archive.local_folder_tag(), None);
        assert_eq!(FolderRole::All.local_folder_tag(), None);
        assert_eq!(FolderRole::Other.local_folder_tag(), None);
    }
}
