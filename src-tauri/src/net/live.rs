//! Live transport adapters — compiled only under `--features live-net`.
//!
//! Scope status:
//! * [`LiveTokenEndpoint`] — real OAuth code-exchange / refresh over HTTPS (`reqwest`,
//!   rustls) (T015/T018).
//! * [`LiveConnProbe`] — real connection probe (T014): a TLS IMAP `LOGIN` (which
//!   actually validates the credentials) via `async-imap` over `tokio-rustls`, plus
//!   an SMTP reachability check via `lettre`.
//! * [`LiveImapFactory`] — opens a real authenticated streaming IMAP **session**
//!   ([`LiveImapSession`]) implementing SELECT / UID SEARCH / UID FETCH for sync
//!   plus per-attachment part fetch (T021/T022/T025).

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

    fn fetch_part(&mut self, uid: i64, part_index: u32) -> BoxFuture<'_, AppResult<Vec<u8>>> {
        Box::pin(async move {
            // Fetch the full RFC-822 message and slice out the requested attachment
            // by re-parsing it. Addressing by the parser's own attachment index
            // (the value stored at ingest) keeps both sides in lock-step without
            // re-deriving IMAP BODYSTRUCTURE part numbers, which mail-parser does
            // not expose and which are error-prone for nested multiparts.
            let mut stream = self
                .session
                .uid_fetch(uid.to_string(), "BODY.PEEK[]")
                .await
                .map_err(|e| classify_imap_err(&e))?;
            let mut raw: Option<Vec<u8>> = None;
            while let Some(item) = stream.next().await {
                let fetch = item.map_err(|e| classify_imap_err(&e))?;
                if let Some(body) = fetch.body() {
                    raw = Some(body.to_vec());
                    break;
                }
            }
            let raw =
                raw.ok_or_else(|| AppError::ImapConnection(format!("no body for UID {uid}")))?;
            let msg = mail_parser::MessageParser::default()
                .parse(&raw)
                .ok_or_else(|| {
                    AppError::ImapConnection(format!("unparsable body for UID {uid}"))
                })?;
            // A part the message no longer contains is terminal (e.g. server-side
            // change) — surface NotFound so auto-download stops retrying.
            let part = msg
                .attachments()
                .nth(part_index as usize)
                .ok_or(AppError::NotFound)?;
            Ok(part.contents().to_vec())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{TokenEndpoint, TokenRequest};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn code_exchange(token_url: String) -> TokenRequest {
        TokenRequest {
            token_url,
            client_id: "client-abc".into(),
            redirect_uri: "http://127.0.0.1:0/cb".into(),
            code: Some("auth-code".into()),
            code_verifier: Some("pkce-verifier".into()),
            refresh_token: None,
            scope: Some("offline_access".into()),
        }
    }

    /// A minimal, tag-echoing IMAP server scripted for one INBOX with three
    /// messages (UIDs 2, 3, 5). It speaks just enough of RFC-3501 to drive
    /// [`LiveImapSession`] through LOGIN → SELECT → UID SEARCH → UID FETCH over a
    /// plain (non-TLS) socket. Returns the bound port.
    async fn spawn_scripted_imap() -> u16 {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let (rd, mut wr) = sock.split();
            let mut reader = BufReader::new(rd);
            // Untagged greeting before any command.
            wr.write_all(b"* OK IMAP4rev1 Service Ready\r\n")
                .await
                .unwrap();

            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).await.unwrap() == 0 {
                    break; // client closed the connection
                }
                let trimmed = line.trim_end();
                let mut parts = trimmed.splitn(3, ' ');
                let tag = parts.next().unwrap_or("").to_string();
                let cmd = parts.next().unwrap_or("").to_ascii_uppercase();
                let rest = parts.next().unwrap_or("");

                match cmd.as_str() {
                    "LOGIN" => {
                        wr.write_all(format!("{tag} OK LOGIN completed\r\n").as_bytes())
                            .await
                            .unwrap();
                    }
                    "SELECT" => {
                        wr.write_all(
                            b"* 3 EXISTS\r\n* 0 RECENT\r\n\
                              * OK [UIDVALIDITY 42] UIDs valid\r\n\
                              * OK [UIDNEXT 6] Predicted next UID\r\n",
                        )
                        .await
                        .unwrap();
                        wr.write_all(
                            format!("{tag} OK [READ-WRITE] SELECT completed\r\n").as_bytes(),
                        )
                        .await
                        .unwrap();
                    }
                    "UID" => {
                        let sub = rest.split(' ').next().unwrap_or("").to_ascii_uppercase();
                        if sub == "SEARCH" {
                            wr.write_all(b"* SEARCH 2 3 5\r\n").await.unwrap();
                            wr.write_all(format!("{tag} OK SEARCH completed\r\n").as_bytes())
                                .await
                                .unwrap();
                        } else {
                            // UID FETCH: emit three literal-bearing FETCH responses.
                            for (seq, uid, body) in [
                                (1u32, 2u32, &b"raw-2"[..]),
                                (2, 3, &b"raw-3"[..]),
                                (3, 5, &b"raw-5"[..]),
                            ] {
                                wr.write_all(
                                    format!(
                                        "* {seq} FETCH (UID {uid} BODY[] {{{}}}\r\n",
                                        body.len()
                                    )
                                    .as_bytes(),
                                )
                                .await
                                .unwrap();
                                wr.write_all(body).await.unwrap();
                                wr.write_all(b")\r\n").await.unwrap();
                            }
                            wr.write_all(format!("{tag} OK FETCH completed\r\n").as_bytes())
                                .await
                                .unwrap();
                        }
                    }
                    "LOGOUT" => {
                        wr.write_all(b"* BYE\r\n").await.unwrap();
                        wr.write_all(format!("{tag} OK LOGOUT completed\r\n").as_bytes())
                            .await
                            .unwrap();
                        break;
                    }
                    _ => {
                        wr.write_all(format!("{tag} OK\r\n").as_bytes())
                            .await
                            .unwrap();
                    }
                }
            }
        });
        port
    }

    #[tokio::test]
    async fn live_imap_session_runs_select_search_fetch_over_a_socket() {
        use crate::net::{ImapCreds, ImapFactory};

        let port = spawn_scripted_imap().await;
        let creds = ImapCreds {
            host: "127.0.0.1".into(),
            port,
            tls: false, // plain socket — exercise the non-TLS LiveImapFactory branch
            email: "you@example.com".into(),
            secret: "app-password".into(),
        };

        let mut session = LiveImapFactory::new().open(creds).await.unwrap();

        let status = session.select_inbox().await.unwrap();
        assert_eq!(status.uid_validity, 42);
        assert_eq!(status.uid_next, 6);
        assert_eq!(status.exists, 3);

        // SINCE search → newest (highest UID) first.
        let uids = session.search_uids_since(0).await.unwrap();
        assert_eq!(uids, vec![5, 3, 2]);

        // FETCH the bodies and confirm UID→bytes pairing survives the wire parse.
        let bodies = session.fetch_bodies(&[2, 3, 5]).await.unwrap();
        assert_eq!(bodies.len(), 3);
        assert!(bodies.iter().any(|(u, b)| *u == 2 && b == b"raw-2"));
        assert!(bodies.iter().any(|(u, b)| *u == 5 && b == b"raw-5"));
    }

    #[tokio::test]
    async fn code_exchange_parses_access_refresh_and_expiry() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "at-123",
                "refresh_token": "rt-456",
                "expires_in": 1200,
                "token_type": "Bearer"
            })))
            .mount(&server)
            .await;

        let resp = LiveTokenEndpoint::new()
            .exchange(code_exchange(format!("{}/token", server.uri())))
            .await
            .unwrap();
        assert_eq!(resp.access_token, "at-123");
        assert_eq!(resp.refresh_token.as_deref(), Some("rt-456"));
        assert_eq!(resp.expires_in_secs, 1200);
    }

    #[tokio::test]
    async fn refresh_grant_succeeds_and_expiry_defaults_when_absent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            // No `expires_in` and no `refresh_token` in the response.
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "access_token": "at-only" })),
            )
            .mount(&server)
            .await;

        let req = TokenRequest {
            token_url: format!("{}/token", server.uri()),
            client_id: "client-abc".into(),
            redirect_uri: "http://127.0.0.1:0/cb".into(),
            code: None,
            code_verifier: None,
            refresh_token: Some("old-refresh".into()),
            scope: None,
        };
        let resp = LiveTokenEndpoint::new().exchange(req).await.unwrap();
        assert_eq!(resp.access_token, "at-only");
        assert!(resp.refresh_token.is_none());
        // Missing `expires_in` falls back to the one-hour default.
        assert_eq!(resp.expires_in_secs, 3600);
    }

    #[tokio::test]
    async fn non_2xx_status_is_oauth_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "invalid_grant"
            })))
            .mount(&server)
            .await;

        let err = LiveTokenEndpoint::new()
            .exchange(code_exchange(format!("{}/token", server.uri())))
            .await
            .unwrap_err();
        match err {
            AppError::AuthOAuthFailed(msg) => assert!(msg.contains("400"), "got: {msg}"),
            other => panic!("expected AuthOAuthFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_code_and_refresh_fails_without_a_request() {
        // No mail server is contacted: the grant type can't be determined.
        let req = TokenRequest {
            token_url: "http://127.0.0.1:1/token".into(),
            client_id: "client-abc".into(),
            redirect_uri: "http://127.0.0.1:0/cb".into(),
            code: None,
            code_verifier: None,
            refresh_token: None,
            scope: None,
        };
        let err = LiveTokenEndpoint::new().exchange(req).await.unwrap_err();
        assert!(matches!(err, AppError::AuthOAuthFailed(_)));
    }

    #[test]
    fn tls_connector_builds_with_the_pinned_provider() {
        // Exercises the rustls/ring wiring (panics if the provider can't supply
        // the safe default protocol versions).
        let _connector = tls_connector();
    }
}
