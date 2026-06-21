//! In-memory transport fakes — the explicit test seam promised by the module
//! header (T014/T015/T021 §8). Compiled only under `#[cfg(test)]`.
//!
//! These let the service layers above [`Net`] (sync, sampler, backfill, the
//! account probe, OAuth refresh) be driven through their **success** paths
//! without a live server — the offline adapters can only exercise the failure
//! paths. A test scripts a [`FakeMailbox`], wraps it in a [`FakeImapFactory`],
//! and injects the bundle via [`AppState::test_state_with_net`].
//!
//! To assert on what the service did, clone a fake's `log` handle *before*
//! moving it into [`fake_net`] (the `Arc<dyn …>` inside [`Net`] is type-erased
//! and can't be downcast back).
#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;

use super::{
    ConnProbe, ConnProbeConfig, ConnProbeReport, IdleOutcome, ImapCreds, ImapFactory, ImapSession,
    InboxStatus, Net, TokenEndpoint, TokenRequest, TokenResponse,
};
use crate::error::{AppError, AppResult};

/// A shared, ordered record of the transport calls a test made.
pub type CallLog = Arc<Mutex<Vec<String>>>;

/// A scriptable in-memory INBOX. `uids` is the full UID set; `bodies` and
/// `parts` are looked up on demand by [`FakeImapSession`].
#[derive(Clone)]
pub struct FakeMailbox {
    pub inbox: InboxStatus,
    pub uids: Vec<i64>,
    pub bodies: HashMap<i64, Vec<u8>>,
    pub parts: HashMap<(i64, u32), Vec<u8>>,
    /// Scripted IDLE outcomes consumed in order by [`FakeImapSession::idle_wait`];
    /// when empty, `idle_wait` parks for its timeout (mimics a quiet server).
    pub idle_script: Arc<Mutex<VecDeque<IdleOutcome>>>,
}

impl FakeMailbox {
    /// An empty mailbox (UIDVALIDITY 1, UIDNEXT 1, no messages).
    pub fn new() -> Self {
        Self {
            inbox: InboxStatus {
                uid_validity: 1,
                uid_next: 1,
                exists: 0,
            },
            uids: Vec::new(),
            bodies: HashMap::new(),
            parts: HashMap::new(),
            idle_script: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub fn with_inbox(mut self, uid_validity: i64, uid_next: i64, exists: u32) -> Self {
        self.inbox = InboxStatus {
            uid_validity,
            uid_next,
            exists,
        };
        self
    }

    pub fn with_uids(mut self, uids: impl IntoIterator<Item = i64>) -> Self {
        self.uids = uids.into_iter().collect();
        self
    }

    pub fn with_body(mut self, uid: i64, bytes: impl Into<Vec<u8>>) -> Self {
        self.bodies.insert(uid, bytes.into());
        self
    }

    pub fn with_part(mut self, uid: i64, index: u32, bytes: impl Into<Vec<u8>>) -> Self {
        self.parts.insert((uid, index), bytes.into());
        self
    }

    /// Script the outcomes the next `idle_wait` calls return, in order. When the
    /// script is exhausted, `idle_wait` parks for its timeout (a quiet server).
    pub fn with_idle_outcomes(self, outcomes: impl IntoIterator<Item = IdleOutcome>) -> Self {
        self.idle_script.lock().unwrap().extend(outcomes);
        self
    }
}

impl Default for FakeMailbox {
    fn default() -> Self {
        Self::new()
    }
}

/// A live "session" served from a cloned [`FakeMailbox`]. Mirrors the contract of
/// [`super::live::LiveImapSession`]: SELECT, two UID searches, batch body fetch,
/// per-attachment part fetch.
pub struct FakeImapSession {
    mailbox: FakeMailbox,
    log: CallLog,
}

impl ImapSession for FakeImapSession {
    fn select_inbox(&mut self) -> BoxFuture<'_, AppResult<InboxStatus>> {
        self.log.lock().unwrap().push("select_inbox".to_string());
        let inbox = self.mailbox.inbox;
        Box::pin(async move { Ok(inbox) })
    }

    fn search_uids_since(&mut self, since_epoch: i64) -> BoxFuture<'_, AppResult<Vec<i64>>> {
        self.log
            .lock()
            .unwrap()
            .push(format!("search_since:{since_epoch}"));
        let mut uids = self.mailbox.uids.clone();
        uids.sort_unstable_by(|a, b| b.cmp(a)); // newest (highest UID) first
        Box::pin(async move { Ok(uids) })
    }

    fn search_uids_from(&mut self, uid_from: i64) -> BoxFuture<'_, AppResult<Vec<i64>>> {
        self.log
            .lock()
            .unwrap()
            .push(format!("search_from:{uid_from}"));
        let mut uids: Vec<i64> = self
            .mailbox
            .uids
            .iter()
            .copied()
            .filter(|&u| u >= uid_from)
            .collect();
        uids.sort_unstable();
        Box::pin(async move { Ok(uids) })
    }

    fn fetch_bodies(&mut self, uids: &[i64]) -> BoxFuture<'_, AppResult<Vec<(i64, Vec<u8>)>>> {
        // Own the result before the async block so it doesn't borrow `uids`/`self`.
        let pairs: Vec<(i64, Vec<u8>)> = uids
            .iter()
            .filter_map(|u| self.mailbox.bodies.get(u).map(|b| (*u, b.clone())))
            .collect();
        Box::pin(async move { Ok(pairs) })
    }

    fn fetch_part(&mut self, uid: i64, part_index: u32) -> BoxFuture<'_, AppResult<Vec<u8>>> {
        // A part the mailbox doesn't have is terminal — mirror the live adapter's
        // `NotFound` so auto-download stops retrying.
        let part = self.mailbox.parts.get(&(uid, part_index)).cloned();
        Box::pin(async move { part.ok_or(AppError::NotFound) })
    }

    fn idle_wait(
        &mut self,
        max_wait: std::time::Duration,
    ) -> BoxFuture<'_, AppResult<IdleOutcome>> {
        self.log.lock().unwrap().push("idle_wait".to_string());
        let next = self.mailbox.idle_script.lock().unwrap().pop_front();
        Box::pin(async move {
            match next {
                Some(outcome) => Ok(outcome),
                // No scripted events left → behave like a quiet server: park until
                // the keepalive window elapses, then report a timeout. Keeps the
                // listener loop from spinning in tests.
                None => {
                    tokio::time::sleep(max_wait).await;
                    Ok(IdleOutcome::TimedOut)
                }
            }
        })
    }
}

/// Opens [`FakeImapSession`]s over a fixed mailbox. `failing` makes every `open`
/// return an `ImapConnection` error (to drive the connect-failure path).
pub struct FakeImapFactory {
    mailbox: FakeMailbox,
    fail: Option<String>,
    /// Shared call log. Clone this handle before moving the factory into
    /// [`fake_net`] to inspect calls afterward.
    pub log: CallLog,
}

impl FakeImapFactory {
    pub fn new(mailbox: FakeMailbox) -> Self {
        Self {
            mailbox,
            fail: None,
            log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// A factory whose every `open` fails with `ImapConnection(message)`.
    pub fn failing(message: impl Into<String>) -> Self {
        Self {
            mailbox: FakeMailbox::new(),
            fail: Some(message.into()),
            log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// The call log handle (shared with every session this factory opened).
    pub fn log(&self) -> CallLog {
        self.log.clone()
    }
}

impl ImapFactory for FakeImapFactory {
    fn open(&self, creds: ImapCreds) -> BoxFuture<'_, AppResult<Box<dyn ImapSession>>> {
        self.log
            .lock()
            .unwrap()
            .push(format!("open:{}", creds.email));
        let fail = self.fail.clone();
        let mailbox = self.mailbox.clone();
        let log = self.log.clone();
        Box::pin(async move {
            if let Some(msg) = fail {
                return Err(AppError::ImapConnection(msg));
            }
            Ok(Box::new(FakeImapSession { mailbox, log }) as Box<dyn ImapSession>)
        })
    }
}

/// Returns a fixed [`ConnProbeReport`] (in-band, never `Err`).
pub struct FakeConnProbe {
    report: ConnProbeReport,
}

impl FakeConnProbe {
    /// Both IMAP and SMTP reachable.
    pub fn ok() -> Self {
        Self {
            report: ConnProbeReport {
                imap_ok: true,
                smtp_ok: true,
                error_message: None,
            },
        }
    }

    pub fn with_report(report: ConnProbeReport) -> Self {
        Self { report }
    }
}

impl ConnProbe for FakeConnProbe {
    fn verify(&self, _cfg: ConnProbeConfig) -> BoxFuture<'_, ConnProbeReport> {
        let report = self.report.clone();
        Box::pin(async move { report })
    }
}

enum TokenOutcome {
    Ok(TokenResponse),
    Err(String),
}

/// Returns a scripted token response (or an OAuth failure) for every exchange.
pub struct FakeTokenEndpoint {
    outcome: TokenOutcome,
}

impl FakeTokenEndpoint {
    pub fn returning(access: &str, refresh: Option<&str>, expires_in_secs: i64) -> Self {
        Self {
            outcome: TokenOutcome::Ok(TokenResponse {
                access_token: access.to_string(),
                refresh_token: refresh.map(String::from),
                expires_in_secs,
            }),
        }
    }

    pub fn failing(message: impl Into<String>) -> Self {
        Self {
            outcome: TokenOutcome::Err(message.into()),
        }
    }
}

impl TokenEndpoint for FakeTokenEndpoint {
    fn exchange(&self, _req: TokenRequest) -> BoxFuture<'_, AppResult<TokenResponse>> {
        let out = match &self.outcome {
            TokenOutcome::Ok(r) => Ok(r.clone()),
            TokenOutcome::Err(m) => Err(AppError::AuthOAuthFailed(m.clone())),
        };
        Box::pin(async move { out })
    }
}

/// Assemble a [`Net`] from fake transports. Any `None` field gets a benign
/// default (empty mailbox / all-ok probe / a dummy token) so a test only builds
/// the transport it cares about.
pub fn fake_net(
    imap: Option<FakeImapFactory>,
    probe: Option<FakeConnProbe>,
    oauth: Option<FakeTokenEndpoint>,
) -> Net {
    Net {
        imap: Arc::new(imap.unwrap_or_else(|| FakeImapFactory::new(FakeMailbox::new()))),
        probe: Arc::new(probe.unwrap_or_else(FakeConnProbe::ok)),
        oauth: Arc::new(
            oauth.unwrap_or_else(|| FakeTokenEndpoint::returning("fake-access", None, 3600)),
        ),
    }
}

/// Shorthand: a [`Net`] whose only non-default transport is the IMAP factory.
pub fn net_with_imap(factory: FakeImapFactory) -> Net {
    fake_net(Some(factory), None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds(email: &str) -> ImapCreds {
        ImapCreds {
            host: "imap.example.com".into(),
            port: 993,
            tls: true,
            email: email.into(),
            secret: "secret".into(),
        }
    }

    #[tokio::test]
    async fn session_round_trips_a_scripted_mailbox() {
        let mailbox = FakeMailbox::new()
            .with_inbox(42, 6, 3)
            .with_uids([2, 3, 5])
            .with_body(2, b"raw-2".to_vec())
            .with_body(5, b"raw-5".to_vec())
            .with_part(5, 0, b"attachment-bytes".to_vec());
        let factory = FakeImapFactory::new(mailbox);
        let log = factory.log();

        let mut session = factory.open(creds("you@example.com")).await.unwrap();

        let inbox = session.select_inbox().await.unwrap();
        assert_eq!(inbox.uid_validity, 42);
        assert_eq!(inbox.uid_next, 6);
        assert_eq!(inbox.exists, 3);

        // search_since returns newest-first; search_from filters and sorts asc.
        assert_eq!(session.search_uids_since(0).await.unwrap(), vec![5, 3, 2]);
        assert_eq!(session.search_uids_from(3).await.unwrap(), vec![3, 5]);

        // Only UIDs that have a stored body come back.
        let bodies = session.fetch_bodies(&[2, 3, 5]).await.unwrap();
        assert_eq!(bodies.len(), 2);
        assert!(bodies.iter().any(|(u, b)| *u == 2 && b == b"raw-2"));
        assert!(bodies.iter().any(|(u, b)| *u == 5 && b == b"raw-5"));

        assert_eq!(
            session.fetch_part(5, 0).await.unwrap(),
            b"attachment-bytes".to_vec()
        );
        // A missing part is NotFound (terminal).
        assert!(matches!(
            session.fetch_part(5, 9).await,
            Err(AppError::NotFound)
        ));

        let calls = log.lock().unwrap().clone();
        assert_eq!(
            calls.first().map(String::as_str),
            Some("open:you@example.com")
        );
        assert!(calls.iter().any(|c| c == "select_inbox"));
    }

    #[tokio::test]
    async fn idle_wait_returns_scripted_outcomes_then_times_out() {
        let mailbox = FakeMailbox::new().with_idle_outcomes([IdleOutcome::MailArrived]);
        let factory = FakeImapFactory::new(mailbox);
        let mut session = factory.open(creds("you@example.com")).await.unwrap();

        // The scripted outcome is consumed first…
        assert_eq!(
            session
                .idle_wait(std::time::Duration::from_millis(5))
                .await
                .unwrap(),
            IdleOutcome::MailArrived
        );
        // …then a quiet server parks for the (tiny) timeout and reports TimedOut.
        assert_eq!(
            session
                .idle_wait(std::time::Duration::from_millis(5))
                .await
                .unwrap(),
            IdleOutcome::TimedOut
        );
    }

    #[tokio::test]
    async fn failing_factory_reports_connection_error() {
        let factory = FakeImapFactory::failing("refused");
        // `Box<dyn ImapSession>` isn't `Debug`, so match the Result rather than
        // calling `unwrap_err` (which would require the Ok type to be `Debug`).
        let result = factory.open(creds("x@y.com")).await;
        assert!(matches!(result, Err(AppError::ImapConnection(_))));
    }

    #[tokio::test]
    async fn probe_and_token_fakes_return_scripted_values() {
        let probe = FakeConnProbe::ok();
        let report = probe
            .verify(ConnProbeConfig {
                imap: creds("a@b.com"),
                smtp: super::super::SmtpCreds {
                    host: "smtp".into(),
                    port: 587,
                    tls: true,
                    email: "a@b.com".into(),
                    secret: "s".into(),
                },
            })
            .await;
        assert!(report.imap_ok && report.smtp_ok);

        let ok = FakeTokenEndpoint::returning("acc", Some("ref"), 1200);
        let req = TokenRequest {
            token_url: "https://t".into(),
            client_id: "c".into(),
            redirect_uri: "r".into(),
            code: Some("code".into()),
            code_verifier: Some("v".into()),
            refresh_token: None,
            scope: None,
        };
        let resp = ok.exchange(req.clone()).await.unwrap();
        assert_eq!(resp.access_token, "acc");
        assert_eq!(resp.refresh_token.as_deref(), Some("ref"));
        assert_eq!(resp.expires_in_secs, 1200);

        let bad = FakeTokenEndpoint::failing("nope");
        assert!(matches!(
            bad.exchange(req).await,
            Err(AppError::AuthOAuthFailed(_))
        ));
    }
}
