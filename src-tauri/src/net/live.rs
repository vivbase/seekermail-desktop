//! Live transport adapters — compiled only under `--features live-net`.
//!
//! Scope status:
//! * [`LiveTokenEndpoint`] — real OAuth code-exchange / refresh over HTTPS (`reqwest`,
//!   rustls) (T015/T018).
//! * [`LiveConnProbe`] — real connection probe (T014): a TLS IMAP `LOGIN` (which
//!   actually validates the credentials) via `async-imap` over `tokio-rustls`, plus
//!   an SMTP reachability check via `lettre`.
//! * [`LiveImapFactory`] — the streaming IMAP **session** (SELECT / SEARCH / FETCH for
//!   sync) is the remaining binding point; until it lands the probe still proves real
//!   connectivity and the service/command layers above this seam are untouched.

use std::sync::Arc;

use futures::future::BoxFuture;
use futures::StreamExt;
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

use super::{
    ConnProbe, ConnProbeConfig, ConnProbeReport, ImapCreds, ImapFactory, ImapSession, InboxStatus,
    SmtpCreds, TokenEndpoint, TokenRequest, TokenResponse,
};
use crate::error::{AppError, AppResult};

/// OAuth token endpoint over HTTPS (`reqwest`, rustls).
pub struct LiveTokenEndpoint {
    client: reqwest::Client,
}

impl LiveTokenEndpoint {
    pub fn new() -> Self {
        // A short, fixed timeout keeps a hung provider from wedging a sync task.
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap_or_default();
        Self { client }
    }
}

#[derive(serde::Deserialize)]
struct RawTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

impl TokenEndpoint for LiveTokenEndpoint {
    fn exchange(&self, req: TokenRequest) -> BoxFuture<'_, AppResult<TokenResponse>> {
        let client = self.client.clone();
        Box::pin(async move {
            // Build the x-www-form-urlencoded body for either grant type.
            let mut form: Vec<(&str, String)> = vec![
                ("client_id", req.client_id.clone()),
                ("redirect_uri", req.redirect_uri.clone()),
            ];
            if let (Some(code), Some(verifier)) = (&req.code, &req.code_verifier) {
                form.push(("grant_type", "authorization_code".into()));
                form.push(("code", code.clone()));
                form.push(("code_verifier", verifier.clone()));
            } else if let Some(refresh) = &req.refresh_token {
                form.push(("grant_type", "refresh_token".into()));
                form.push(("refresh_token", refresh.clone()));
            } else {
                return Err(AppError::AuthOAuthFailed(
                    "missing code or refresh_token".into(),
                ));
            }
            if let Some(scope) = &req.scope {
                form.push(("scope", scope.clone()));
            }

            let resp = client
                .post(&req.token_url)
                .form(&form)
                .send()
                .await
                .map_err(|e| AppError::AuthOAuthFailed(format!("token request failed: {e}")))?;

            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                return Err(AppError::AuthOAuthFailed(format!(
                    "token endpoint http {status}"
                )));
            }

            let raw: RawTokenResponse = resp
                .json()
                .await
                .map_err(|e| AppError::AuthOAuthFailed(format!("token decode failed: {e}")))?;

            Ok(TokenResponse {
                access_token: raw.access_token,
                refresh_token: raw.refresh_token,
                expires_in_secs: raw.expires_in.unwrap_or(3600),
            })
        })
    }
}

// ── TLS + IMAP/SMTP probe helpers ────────────────────────────────────────────

/// Build a rustls TLS connector trusting the bundled Mozilla webpki root set.
/// Pins the `ring` crypto provider explicitly so the build never depends on a
/// process-global default provider being installed elsewhere.
fn tls_connector() -> TlsConnector {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder_with_provider(Arc::new(
        tokio_rustls::rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("ring provider supports the safe default protocol versions")
    .with_root_certificates(roots)
    .with_no_client_auth();
    TlsConnector::from(Arc::new(config))
}

/// Wrap a connected stream in an IMAP client, consume the server greeting, then
/// `LOGIN`. Succeeds only when the server accepts the credentials; logs out after.
async fn imap_login_check<T>(stream: T, email: &str, secret: &str) -> Result<(), String>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let mut client = async_imap::Client::new(stream);
    // The server sends an untagged "* OK ..." greeting on connect; consume it
    // before issuing LOGIN so the command/response pairing stays aligned.
    let _ = client.read_response().await;
    match client.login(email, secret).await {
        Ok(mut session) => {
            let _ = session.logout().await;
            Ok(())
        }
        // `login` hands the client back on failure; we only need the error.
        Err((e, _client)) => Err(format!("{e}")),
    }
}

/// Open a TCP (optionally TLS) connection to the IMAP host and validate the
/// credentials with a real `LOGIN`.
async fn imap_probe(creds: &ImapCreds) -> Result<(), String> {
    let tcp = TcpStream::connect((creds.host.as_str(), creds.port))
        .await
        .map_err(|e| format!("connect {}:{}: {e}", creds.host, creds.port))?;
    if creds.tls {
        let domain = ServerName::try_from(creds.host.clone())
            .map_err(|e| format!("invalid TLS server name {}: {e}", creds.host))?;
        let tls = tls_connector()
            .connect(domain, tcp)
            .await
            .map_err(|e| format!("TLS handshake: {e}"))?;
        imap_login_check(tls, &creds.email, &creds.secret).await
    } else {
        imap_login_check(tcp, &creds.email, &creds.secret).await
    }
}

/// Confirm the SMTP host is reachable and speaks (STARTTLS) SMTP. `lettre`'s
/// `test_connection` does CONNECT + EHLO + STARTTLS + NOOP + QUIT.
async fn smtp_probe(creds: &SmtpCreds) -> Result<(), String> {
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{AsyncSmtpTransport, Tokio1Executor};

    // Port 465 → implicit TLS (`relay`); anything else (587) → STARTTLS.
    let builder = if creds.port == 465 {
        AsyncSmtpTransport::<Tokio1Executor>::relay(&creds.host)
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&creds.host)
    }
    .map_err(|e| format!("SMTP relay setup: {e}"))?;

    let transport: AsyncSmtpTransport<Tokio1Executor> = builder
        .port(creds.port)
        .credentials(Credentials::new(creds.email.clone(), creds.secret.clone()))
        .build();

    match transport.test_connection().await {
        Ok(true) => Ok(()),
        Ok(false) => Err("server refused the connection".into()),
        Err(e) => Err(format!("{e}")),
    }
}

/// Live IMAP factory — opens a real authenticated streaming session. TLS for
/// port 993 (the standard), plain TCP otherwise. Credentials are validated by the
/// `LOGIN` inside [`imap_login`].
pub struct LiveImapFactory;

impl LiveImapFactory {
    pub fn new() -> Self {
        Self
    }
}

impl ImapFactory for LiveImapFactory {
    fn open(&self, creds: ImapCreds) -> BoxFuture<'_, AppResult<Box<dyn ImapSession>>> {
        Box::pin(async move {
            let tcp = TcpStream::connect((creds.host.as_str(), creds.port))
                .await
                .map_err(|e| {
                    AppError::ImapConnection(format!("connect {}:{}: {e}", creds.host, creds.port))
                })?;
            if creds.tls {
                let domain = ServerName::try_from(creds.host.clone()).map_err(|e| {
                    AppError::ImapConnection(format!("invalid TLS server name {}: {e}", creds.host))
                })?;
                let tls = tls_connector()
                    .connect(domain, tcp)
                    .await
                    .map_err(|e| AppError::ImapConnection(format!("TLS handshake: {e}")))?;
                let session = imap_login(tls, &creds.email, &creds.secret).await?;
                Ok(Box::new(LiveImapSession { session }) as Box<dyn ImapSession>)
            } else {
                let session = imap_login(tcp, &creds.email, &creds.secret).await?;
                Ok(Box::new(LiveImapSession { session }) as Box<dyn ImapSession>)
            }
        })
    }
}

/// Open an IMAP client over `stream`, consume the server greeting, then `LOGIN`.
/// Returns the authenticated session, or an auth/connection error.
async fn imap_login<S>(stream: S, email: &str, secret: &str) -> AppResult<async_imap::Session<S>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send + 'static,
{
    let mut client = async_imap::Client::new(stream);
    // The server sends an untagged "* OK ..." greeting on connect; consume it
    // before issuing LOGIN so the command/response pairing stays aligned.
    let _ = client.read_response().await;
    match client.login(email, secret).await {
        Ok(session) => Ok(session),
        // `login` hands the client back on failure; we only need the error.
        Err((e, _client)) => Err(classify_imap_err(&e)),
    }
}

/// Map an async-imap error to the app error model. A `NO`/`BAD` response to a
/// LOGIN (or a SASL `Authentication` failure) means the credentials are wrong —
/// surfaced as [`AppError::AuthInvalidCredentials`] so the poll loop stops
/// instead of backing off and retrying forever. Everything else is transient.
fn classify_imap_err(e: &async_imap::error::Error) -> AppError {
    use async_imap::error::Error as E;
    match e {
        E::No(_) | E::Bad(_) => AppError::AuthInvalidCredentials,
        other => AppError::ImapConnection(format!("{other}")),
    }
}

/// A live, authenticated IMAP session bound to a single connection. Implements
/// the [`ImapSession`] seam over `async-imap`'s `Session` (SELECT / UID SEARCH /
/// UID FETCH).
struct LiveImapSession<S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    session: async_imap::Session<S>,
}

impl<S> ImapSession for LiveImapSession<S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send + 'static,
{
    fn select_inbox(&mut self) -> BoxFuture<'_, AppResult<InboxStatus>> {
        Box::pin(async move {
            let mb = self
                .session
                .select("INBOX")
                .await
                .map_err(|e| classify_imap_err(&e))?;
            Ok(InboxStatus {
                uid_validity: mb.uid_validity.unwrap_or(0) as i64,
                uid_next: mb.uid_next.unwrap_or(1) as i64,
                exists: mb.exists,
            })
        })
    }

    fn search_uids_since(&mut self, since_epoch: i64) -> BoxFuture<'_, AppResult<Vec<i64>>> {
        Box::pin(async move {
            let date = chrono::DateTime::from_timestamp(since_epoch, 0)
                .unwrap_or_else(chrono::Utc::now)
                .format("%d-%b-%Y");
            let set = self
                .session
                .uid_search(format!("SINCE {date}"))
                .await
                .map_err(|e| classify_imap_err(&e))?;
            let mut uids: Vec<i64> = set.into_iter().map(|u| u as i64).collect();
            uids.sort_unstable_by(|a, b| b.cmp(a)); // newest (highest UID) first
            Ok(uids)
        })
    }

    fn search_uids_from(&mut self, uid_from: i64) -> BoxFuture<'_, AppResult<Vec<i64>>> {
        Box::pin(async move {
            let from = uid_from.max(1);
            let set = self
                .session
                .uid_search(format!("UID {from}:*"))
                .await
                .map_err(|e| classify_imap_err(&e))?;
            // `from:*` always returns at least the highest UID even when it is
            // below `from`; filter so a fully-synced mailbox yields nothing.
            let mut uids: Vec<i64> = set
                .into_iter()
                .map(|u| u as i64)
                .filter(|&u| u >= from)
                .collect();
            uids.sort_unstable();
            Ok(uids)
        })
    }

    fn fetch_bodies(&mut self, uids: &[i64]) -> BoxFuture<'_, AppResult<Vec<(i64, Vec<u8>)>>> {
        // Own the UID set before the async block so we don't borrow `uids` past
        // the future (the future only borrows `self`).
        let set = uids
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");
        Box::pin(async move {
            let mut out = Vec::new();
            if set.is_empty() {
                return Ok(out);
            }
            let mut stream = self
                .session
                .uid_fetch(set, "BODY.PEEK[]")
                .await
                .map_err(|e| classify_imap_err(&e))?;
            while let Some(item) = stream.next().await {
                let fetch = item.map_err(|e| classify_imap_err(&e))?;
                if let (Some(uid), Some(body)) = (fetch.uid, fetch.body()) {
                    out.push((uid as i64, body.to_vec()));
                }
            }
            Ok(out)
        })
    }

    fn fetch_part(&mut self, uid: i64, part: &str) -> BoxFuture<'_, AppResult<Vec<u8>>> {
        let part = part.to_string();
        Box::pin(async move {
            let mut stream = self
                .session
                .uid_fetch(uid.to_string(), format!("BODY.PEEK[{part}]"))
                .await
                .map_err(|e| classify_imap_err(&e))?;
            let mut bytes: Option<Vec<u8>> = None;
            while let Some(item) = stream.next().await {
                let fetch = item.map_err(|e| classify_imap_err(&e))?;
                if let Some(data) = fetch.body().or_else(|| fetch.text()) {
                    bytes = Some(data.to_vec());
                    break;
                }
            }
            bytes.ok_or_else(|| {
                AppError::ImapConnection(format!("no data for UID {uid} part {part}"))
            })
        })
    }
}

/// Live connection probe (T014): real IMAP `LOGIN` + SMTP reachability check.
pub struct LiveConnProbe;

impl LiveConnProbe {
    pub fn new() -> Self {
        Self
    }
}

impl ConnProbe for LiveConnProbe {
    fn verify(&self, cfg: ConnProbeConfig) -> BoxFuture<'_, ConnProbeReport> {
        Box::pin(async move {
            let imap_res = imap_probe(&cfg.imap).await;
            let smtp_res = smtp_probe(&cfg.smtp).await;

            let imap_ok = imap_res.is_ok();
            let smtp_ok = smtp_res.is_ok();

            let mut errs = Vec::new();
            if let Err(e) = imap_res {
                errs.push(format!("IMAP: {e}"));
            }
            if let Err(e) = smtp_res {
                errs.push(format!("SMTP: {e}"));
            }

            ConnProbeReport {
                imap_ok,
                smtp_ok,
                error_message: (!errs.is_empty()).then(|| errs.join("; ")),
            }
        })
    }
}
