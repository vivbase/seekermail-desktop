//! SMTP send service + 10-second cancel window (T043).
//!
//! `schedule_send` returns immediately with a `pendingId`; the actual delivery
//! happens 10 s later inside a spawned task that races a `oneshot` cancel signal
//! (`tokio::select!`, the spec's design — no polling). `cancel_send` fires that
//! signal. On delivery the mail is written to the local `mails` table (folder
//! `SENT`) and a `mail:new` event fires.
//!
//! ## Transport seam (mirrors `net::*`)
//! The real SMTP transport (`lettre`) is heavy + network-bound, so — exactly like
//! IMAP/OAuth (`net/mod.rs`) — it lives behind `--features live-net`. A feature-less
//! build cannot transmit: under `cargo test` the offline transport accepts (no
//! network) so the cancel window, SENT persistence, and `mail:new` event stay
//! exercisable; in a real feature-less binary it instead returns an error, so a
//! stock `cargo build` can never silently pretend a message was sent.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::oneshot;

use crate::error::{AppError, AppResult};
use crate::net::SmtpCreds;
use crate::state::AppState;
use crate::storage::{mail_writer, AccountRepo};
use crate::types::{Account, CancelSendResult, Recipient, SendMailParams, SendMailResult};
use crate::util::new_uuid;

/// The cancellation window before a queued message is actually sent (T043 §6).
pub const SEND_CANCEL_WINDOW_SECS: u64 = 10;

/// In-memory registry of cancellable pending sends. Cloneable; lives in `AppState`.
#[derive(Clone, Default)]
pub struct SendQueue {
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<()>>>>,
}

impl SendQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancel a pending send. `true` if it was still within the window.
    pub fn cancel(&self, pending_id: &str) -> CancelSendResult {
        let removed = self
            .pending
            .lock()
            .expect("send queue poisoned")
            .remove(pending_id);
        match removed {
            Some(tx) => {
                let _ = tx.send(()); // wake the select! cancel arm
                CancelSendResult { cancelled: true }
            }
            None => CancelSendResult { cancelled: false },
        }
    }

    fn register(&self, id: String, tx: oneshot::Sender<()>) {
        self.pending
            .lock()
            .expect("send queue poisoned")
            .insert(id, tx);
    }

    fn unregister(&self, id: &str) {
        self.pending.lock().expect("send queue poisoned").remove(id);
    }
}

/// Normalised outbound message handed to the transport.
#[derive(Debug, Clone)]
pub struct OutboundMessage {
    pub from_name: String,
    pub from_email: String,
    pub to: Vec<Recipient>,
    pub cc: Vec<Recipient>,
    pub bcc: Vec<Recipient>,
    pub subject: String,
    pub body_text: String,
    pub body_html: Option<String>,
    pub message_id: String,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
}

/// Validate, queue, and schedule a send. Returns the pending id + message id; the
/// message leaves [`SEND_CANCEL_WINDOW_SECS`] later unless cancelled.
pub async fn schedule_send(state: &AppState, params: SendMailParams) -> AppResult<SendMailResult> {
    let account = AccountRepo::new(state.storage.db())
        .get(&params.account_id)
        .await?;
    if !account.is_active {
        return Err(AppError::Forbidden("account is disabled".into()));
    }
    if params.to.is_empty() {
        return Err(AppError::Validation(
            "at least one recipient is required".into(),
        ));
    }

    let pending_id = new_uuid();
    let message_id = make_message_id(&account.email);
    let (tx, rx) = oneshot::channel::<()>();
    state.send_queue.register(pending_id.clone(), tx);

    let st = state.clone();
    let pid = pending_id.clone();
    let mid = message_id.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(SEND_CANCEL_WINDOW_SECS)) => {
                st.send_queue.unregister(&pid);
                if let Err(e) = deliver(&st, &params, &mid).await {
                    tracing::warn!(pending_id = %pid, error = %e, "send failed after cancel window");
                }
            }
            _ = rx => {
                st.send_queue.unregister(&pid);
                tracing::info!(pending_id = %pid, "send cancelled within window");
            }
        }
    });

    Ok(SendMailResult {
        pending_id,
        message_id,
    })
}

/// Cancel a pending send (thin pass-through used by the command).
pub fn cancel_send(state: &AppState, pending_id: &str) -> CancelSendResult {
    state.send_queue.cancel(pending_id)
}

/// Deliver + persist + notify (runs after the window). `pub(crate)` for the
/// E3 send queue (T085), which manages its own 30 s undo window in the DB and
/// must NOT stack `schedule_send`'s additional 10 s window on top.
pub(crate) async fn deliver(
    state: &AppState,
    params: &SendMailParams,
    message_id: &str,
) -> AppResult<()> {
    let account = AccountRepo::new(state.storage.db())
        .get(&params.account_id)
        .await?;
    let creds = SmtpCreds {
        host: account.smtp_host.clone().unwrap_or_default(),
        port: account.smtp_port,
        tls: account.smtp_port != 25,
        email: account.email.clone(),
        secret: read_smtp_secret(state, &params.account_id),
    };
    let msg = build_message(&account, params, message_id);

    transport_send(&creds, &msg).await?;

    let date_sent = crate::util::now_unix();
    let summary =
        mail_writer::write_sent_mail(state.storage.db(), &account, params, message_id, date_sent)
            .await?;

    // Finalising a draft? Remove it now that the mail is in SENT (T045).
    if let Some(draft_id) = &params.draft_id {
        let _ = crate::storage::draft_repo::delete(state.storage.db(), draft_id).await;
    }

    state.events.mail_new(summary);
    Ok(())
}

fn build_message(account: &Account, params: &SendMailParams, message_id: &str) -> OutboundMessage {
    OutboundMessage {
        from_name: account.display_name.clone(),
        from_email: account.email.clone(),
        to: params.to.clone(),
        cc: params.cc.clone(),
        bcc: params.bcc.clone(),
        subject: params.subject.clone(),
        body_text: params.body_text.clone(),
        body_html: params.body_html.clone(),
        message_id: message_id.to_string(),
        in_reply_to: params.in_reply_to.clone(),
        references: params.references.clone(),
    }
}

/// Read the SMTP password from the Keychain (best-effort; the offline transport
/// ignores it). Never logged.
fn read_smtp_secret(state: &AppState, account_id: &str) -> String {
    use crate::keychain::CredKind;
    let Ok(uuid) = uuid::Uuid::parse_str(account_id) else {
        return String::new();
    };
    state
        .keychain
        .get(&uuid, CredKind::SmtpPassword)
        .ok()
        .flatten()
        .map(|s| s.expose().to_string())
        .unwrap_or_default()
}

/// The sending domain for outgoing Message-IDs and the SMTP HELO name, derived
/// from the account address. Falls back to a stable, routable product domain if
/// the address has no usable domain part (it always should in practice).
fn sender_domain(email: &str) -> String {
    match email.rsplit_once('@') {
        Some((_, domain)) if !domain.trim().is_empty() => domain.trim().to_ascii_lowercase(),
        _ => "seekermail.app".to_string(),
    }
}

/// Build an RFC 5322 Message-ID (angle-bracketed) anchored to the sender's own
/// domain. Replaces the legacy non-routable `@seekermail.local` id, which mail
/// providers score as a spam signal and which broke local/wire id parity.
pub(crate) fn make_message_id(from_email: &str) -> String {
    format!("<{}@{}>", new_uuid(), sender_domain(from_email))
}

// ── Transport (offline default / lettre under live-net) ──────────────────────

// Offline build in a REAL binary (not a test): refuse to send. A feature-less
// `cargo build` must never silently pretend a message went out — nothing here can
// reach the network. The shipped product is always built with `--features
// live-net`; this guard only fires for a developer build that forgot the flag.
#[cfg(all(not(feature = "live-net"), not(test)))]
async fn transport_send(_creds: &SmtpCreds, msg: &OutboundMessage) -> AppResult<()> {
    tracing::error!(event = "smtp_send", mode = "offline", message_id = %msg.message_id, "refusing to send: offline build — nothing was transmitted (rebuild with --features live-net)");
    Err(AppError::SmtpSend(
        "offline build: refusing to send — nothing was transmitted (rebuild with --features live-net)"
            .into(),
    ))
}

// Offline build under `cargo test`: accept without a network so the scheduling,
// SENT persistence, and `mail:new` event of the send pipeline stay exercisable in
// unit tests. This stub is compiled out of every shipped binary.
#[cfg(all(not(feature = "live-net"), test))]
async fn transport_send(_creds: &SmtpCreds, msg: &OutboundMessage) -> AppResult<()> {
    tracing::warn!(event = "smtp_send", mode = "offline-test", message_id = %msg.message_id, "offline test stub: message NOT transmitted");
    Ok(())
}

#[cfg(feature = "live-net")]
async fn transport_send(creds: &SmtpCreds, msg: &OutboundMessage) -> AppResult<()> {
    use lettre::message::{header::ContentType, Mailbox, MultiPart, SinglePart};
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::transport::smtp::extension::ClientId;
    use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

    let parse_mbox = |r: &Recipient| -> AppResult<Mailbox> {
        let addr = r
            .email
            .parse()
            .map_err(|e| AppError::Validation(format!("bad address {}: {e}", r.email)))?;
        Ok(Mailbox::new(r.name.clone(), addr))
    };

    let from = format!("{} <{}>", msg.from_name, msg.from_email)
        .parse::<Mailbox>()
        .map_err(|e| AppError::Validation(format!("bad from: {e}")))?;

    let mut builder = Message::builder().from(from).subject(&msg.subject);
    for r in &msg.to {
        builder = builder.to(parse_mbox(r)?);
    }
    for r in &msg.cc {
        builder = builder.cc(parse_mbox(r)?);
    }
    for r in &msg.bcc {
        builder = builder.bcc(parse_mbox(r)?);
    }

    // Identity + threading headers (T043): a routable Message-ID, the reply
    // chain, and a client User-Agent. Providers penalise mail that omits these
    // or carries a non-routable id, and replies must thread on the recipient's
    // side. Stored ids already carry angle brackets, so they are passed verbatim.
    builder = builder.message_id(Some(msg.message_id.clone()));
    if let Some(in_reply_to) = &msg.in_reply_to {
        builder = builder.in_reply_to(in_reply_to.clone());
    }
    if let Some(references) = &msg.references {
        builder = builder.references(references.clone());
    }
    builder = builder.user_agent(format!("SeekerMail/{}", env!("CARGO_PKG_VERSION")));

    let email = match &msg.body_html {
        Some(html) => builder.multipart(
            MultiPart::alternative()
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_PLAIN)
                        .body(msg.body_text.clone()),
                )
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_HTML)
                        .body(html.clone()),
                ),
        ),
        None => builder
            .header(ContentType::TEXT_PLAIN)
            .body(msg.body_text.clone()),
    }
    .map_err(|e| AppError::SmtpSend(format!("build message: {e}")))?;

    let creds_obj = Credentials::new(creds.email.clone(), creds.secret.clone());
    let mailer = if creds.port == 465 {
        AsyncSmtpTransport::<Tokio1Executor>::relay(&creds.host)
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&creds.host)
    }
    .map_err(|e| AppError::SmtpSend(format!("connect: {e}")))?
    .port(creds.port)
    .credentials(creds_obj)
    .hello_name(ClientId::Domain(sender_domain(&creds.email)))
    .build();

    mailer.send(email).await.map_err(map_lettre_err)?;
    Ok(())
}

#[cfg(feature = "live-net")]
fn map_lettre_err(e: lettre::transport::smtp::Error) -> AppError {
    // Transient 4xx (greylisting / rate) → SmtpRateLimited; everything else fails.
    if e.is_transient() {
        AppError::SmtpRateLimited
    } else {
        AppError::SmtpSend(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn account(state: &AppState, active: bool) {
        sqlx::query(
            "INSERT INTO accounts (id,email,display_name,provider,smtp_host,smtp_port,color_token,badge_label,is_active,created_at,updated_at) \
             VALUES ('acc','me@x.com','Me','imap','smtp.x.com',587,'slate','W',?,0,0)",
        )
        .bind(active as i64)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    fn params() -> SendMailParams {
        SendMailParams {
            account_id: "acc".into(),
            to: vec![Recipient {
                name: None,
                email: "bob@x.com".into(),
            }],
            cc: vec![],
            bcc: vec![],
            subject: "Hello".into(),
            body_text: "Hi Bob".into(),
            body_html: None,
            in_reply_to: None,
            references: None,
            draft_id: None,
        }
    }

    #[tokio::test]
    async fn inactive_account_is_forbidden() {
        let (state, _rx) = AppState::test_state().await;
        account(&state, false).await;
        let r = schedule_send(&state, params()).await;
        assert!(matches!(r.unwrap_err(), AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn empty_recipients_is_validation() {
        let (state, _rx) = AppState::test_state().await;
        account(&state, true).await;
        let mut p = params();
        p.to.clear();
        assert!(matches!(
            schedule_send(&state, p).await.unwrap_err(),
            AppError::Validation(_)
        ));
    }

    // NB: these two tests don't use `start_paused`, because a frozen clock stalls
    // the SQLite pool ("pool timed out"). Instead the DB is built under real time;
    // only the 10 s window sleep is fast-forwarded, then time resumes so the
    // spawned deliver task's DB writes run normally.
    #[tokio::test]
    async fn delivers_after_window_and_persists_sent() {
        let (state, _rx) = AppState::test_state().await;
        account(&state, true).await;
        let res = schedule_send(&state, params()).await.unwrap();
        assert!(res.message_id.starts_with('<') && res.message_id.ends_with('>'));
        assert!(res.message_id.contains("@x.com"));
        assert!(!res.message_id.contains("seekermail.local"));

        // Let the spawned task reach its cancel-window timer (real time) before we
        // fast-forward it; otherwise `advance` fires nothing, the task then sleeps
        // the full real window, and it outlasts the poll loop below.
        tokio::time::sleep(Duration::from_millis(150)).await;
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(SEND_CANCEL_WINDOW_SECS + 1)).await;
        tokio::time::resume();

        // Poll (real time) until the spawned task persists the SENT row.
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(5)).await;
            let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM mails WHERE folder='SENT'")
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
            if n == 1 {
                return;
            }
        }
        panic!("sent mail was not persisted after the window");
    }

    #[tokio::test]
    async fn cancel_within_window_prevents_send() {
        let (state, _rx) = AppState::test_state().await;
        account(&state, true).await;
        let res = schedule_send(&state, params()).await.unwrap();
        let c = cancel_send(&state, &res.pending_id);
        assert!(c.cancelled);

        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(SEND_CANCEL_WINDOW_SECS + 1)).await;
        tokio::time::resume();
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM mails WHERE folder='SENT'")
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        assert_eq!(n, 0, "cancelled send must not persist");
    }

    #[tokio::test]
    async fn cancel_unknown_id_is_false() {
        let (state, _rx) = AppState::test_state().await;
        assert!(!cancel_send(&state, "nope").cancelled);
    }
}
