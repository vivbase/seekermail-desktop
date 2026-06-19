//! SeekerMail ID — client-side Google OIDC sign-in (Layer 1, T121, A6).
//!
//! "Sign in with Google" creates the OPTIONAL, mailbox-independent SeekerMail ID.
//! It runs entirely on the client with NO SeekerMail-owned server:
//!
//! 1. [`begin`] generates PKCE, opens a one-shot loopback HTTP listener on
//!    `127.0.0.1:<port>`, and returns the Google authorization URL. The command
//!    layer opens it in the system browser.
//! 2. Google redirects the browser to `http://127.0.0.1:<port>/?code=…&state=…`;
//!    the listener captures the code and hands it back over a channel.
//! 3. [`complete`] exchanges the code (PKCE; a Desktop-app client needs no client
//!    secret) for an `id_token`, verifies it against Google's JWKS, reads the
//!    identity claims, and upserts the local identity row via
//!    [`crate::storage::IdentityRepo::upsert_signin`].
//!
//! Hard redlines (F_A6 §3.7, analysis/26, SECURITY_WHITEPAPER §5A):
//! * Scope is `openid email profile` ONLY — this flow NEVER requests a mail scope
//!   (mailbox access is the separate `account::oauth` grant).
//! * No mail bodies, attachments, contacts, or GTE vectors are touched here.
//!
//! Redirect mechanism: a loopback IP redirect — Google's supported desktop method.
//! The spec's earlier custom-scheme `seekermail://oauth/identity` is rejected by
//! Google's current rules (see knowledge base `docs/analysis/27`). A "Desktop app"
//! OAuth client accepts any `http://127.0.0.1:<port>` redirect with no
//! pre-registration.

mod jwks;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

use crate::account::pkce::new_pkce;
use crate::config::OAUTH_PENDING_TTL_SECS;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage::identity_repo::SignInInput;
use crate::storage::IdentityRepo;
use crate::types::SeekerMailId;
use crate::util::now_unix;

/// Google OIDC authorization endpoint.
const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
/// Google OIDC token endpoint (code → id_token exchange).
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// Identity scope — `openid email profile` ONLY (F_A6 §3.1 hard redline). This
/// flow must NEVER request a mail scope.
const IDENTITY_SCOPE: &str = "openid email profile";
/// Env var holding the Google "Desktop app" OAuth client id (see the dev/15 setup
/// runbook). Read at runtime so the crate always compiles without it.
const CLIENT_ID_ENV: &str = "SEEKERMAIL_GOOGLE_CLIENT_ID";

/// Transient state for an in-flight sign-in, parked in `AppState.identity_oauth`.
/// Mirrors [`crate::account::oauth::PendingOAuth`] but for the identity grant.
pub struct PendingIdentitySignin {
    verifier: String,
    state: String,
    nonce: String,
    redirect_uri: String,
    created_at: i64,
    /// Receives `(code, returned_state)` from the loopback listener thread.
    code_rx: Option<oneshot::Receiver<(String, String)>>,
}

impl PendingIdentitySignin {
    fn is_expired(&self, now: i64) -> bool {
        now - self.created_at > OAUTH_PENDING_TTL_SECS
    }
}

/// Resolve the Google client id from the environment, or a clean error.
fn client_id() -> AppResult<String> {
    std::env::var(CLIENT_ID_ENV).map_err(|_| {
        AppError::AuthOAuthFailed(format!(
            "{CLIENT_ID_ENV} is not configured (see docs/dev/15 setup runbook)"
        ))
    })
}

/// Begin "Sign in with Google": generate PKCE, open a loopback listener, park the
/// pending grant, and return `(authorize_url, state_nonce)`. The command layer
/// opens the URL in the system browser.
pub fn begin(state: &AppState) -> AppResult<(String, String)> {
    let cid = client_id()?;
    let pkce = new_pkce();
    let nonce = rand_nonce();

    // Bind a loopback listener on an OS-assigned free port. The redirect URI sent
    // to Google must match exactly the one used at token exchange, so we store it.
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| AppError::AuthOAuthFailed(format!("loopback bind failed: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| AppError::AuthOAuthFailed(format!("loopback addr failed: {e}")))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}");

    let (tx, rx) = oneshot::channel::<(String, String)>();
    spawn_loopback_listener(listener, tx);

    let url = build_authorize_url(&cid, &pkce.challenge, &pkce.state, &nonce, &redirect_uri);

    let pending = PendingIdentitySignin {
        verifier: pkce.verifier,
        state: pkce.state.clone(),
        nonce,
        redirect_uri,
        created_at: now_unix(),
        code_rx: Some(rx),
    };
    *state
        .identity_oauth
        .lock()
        .expect("identity_oauth mutex poisoned") = Some(pending);

    Ok((url, pkce.state))
}

/// Complete the sign-in: obtain the authorization code (a non-empty `code`
/// argument wins — a manual path; otherwise wait for the loopback listener),
/// exchange it for an `id_token`, verify it against Google's JWKS, and upsert the
/// local identity row. Marketing consent defaults OFF (set later via
/// `set_marketing_consent`).
pub async fn complete(
    state: &AppState,
    code: &str,
    returned_state: &str,
) -> AppResult<SeekerMailId> {
    // Take the pending grant out under the lock, then drop the guard before await.
    let mut pending = {
        let mut guard = state
            .identity_oauth
            .lock()
            .expect("identity_oauth mutex poisoned");
        guard.take()
    }
    .ok_or_else(|| AppError::AuthOAuthFailed("no pending sign-in".into()))?;

    if pending.is_expired(now_unix()) {
        return Err(AppError::AuthOAuthFailed("sign-in expired".into()));
    }
    if pending.state != returned_state {
        return Err(AppError::AuthOAuthFailed("state mismatch".into()));
    }

    // Authoritative authorization code: a non-empty argument (manual path) wins;
    // otherwise wait for the loopback listener to deliver it.
    let auth_code = if !code.is_empty() {
        code.to_string()
    } else {
        let rx = pending
            .code_rx
            .take()
            .ok_or_else(|| AppError::AuthOAuthFailed("sign-in already completed".into()))?;
        let (cb_code, cb_state) =
            tokio::time::timeout(Duration::from_secs(OAUTH_PENDING_TTL_SECS as u64), rx)
                .await
                .map_err(|_| AppError::AuthOAuthFailed("sign-in timed out".into()))?
                .map_err(|_| AppError::AuthOAuthFailed("sign-in was cancelled".into()))?;
        if cb_state != pending.state {
            return Err(AppError::AuthOAuthFailed("loopback state mismatch".into()));
        }
        cb_code
    };

    // Exchange the code (PKCE; Desktop-app client → no client secret).
    let cid = client_id()?;
    let token = exchange_code(&cid, &auth_code, &pending.verifier, &pending.redirect_uri).await?;
    let id_token = token
        .id_token
        .ok_or_else(|| AppError::AuthOAuthFailed("token response had no id_token".into()))?;

    // Verify the id_token against Google's JWKS and read the identity claims.
    let claims = jwks::fetch_and_verify(&id_token, &cid, Some(&pending.nonce)).await?;

    let input = SignInInput {
        provider: "google".into(),
        provider_subject: claims.sub,
        email: claims
            .email
            .ok_or_else(|| AppError::AuthOAuthFailed("id_token had no email".into()))?,
        email_verified: claims.email_verified.unwrap_or(false),
        display_name: claims.name,
        plan: None,
    };

    // Consent defaults OFF at first sign-in (GDPR-safe); onboarding/settings set it
    // explicitly via `set_marketing_consent`.
    IdentityRepo::new(state.storage.db())
        .upsert_signin(&input, false, None)
        .await
}

/// Token-endpoint response — we only need the `id_token` for identity.
#[derive(serde::Deserialize)]
struct TokenResp {
    #[serde(default)]
    id_token: Option<String>,
}

/// POST the authorization-code grant to Google's token endpoint (PKCE; no client
/// secret for a Desktop-app client). Reuses `reqwest` (default build), like the
/// BYO-AI adapters — so sign-in works in a plain `tauri dev` build.
async fn exchange_code(
    client_id: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> AppResult<TokenResp> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| AppError::AuthOAuthFailed(format!("http client: {e}")))?;
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("client_id", client_id),
        ("code_verifier", verifier),
        ("redirect_uri", redirect_uri),
    ];
    let resp = client
        .post(GOOGLE_TOKEN_URL)
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
    resp.json::<TokenResp>()
        .await
        .map_err(|e| AppError::AuthOAuthFailed(format!("token decode failed: {e}")))
}

/// Build the Google authorization URL with PKCE + state + nonce + loopback
/// redirect. `access_type=online` (identity needs no refresh token);
/// `prompt=select_account` lets the user pick which Google account.
fn build_authorize_url(
    client_id: &str,
    challenge: &str,
    state: &str,
    nonce: &str,
    redirect_uri: &str,
) -> String {
    format!(
        "{GOOGLE_AUTH_URL}?response_type=code&client_id={}&redirect_uri={}&scope={}\
         &code_challenge={}&code_challenge_method=S256&state={}&nonce={}\
         &access_type=online&prompt=select_account",
        pct(client_id),
        pct(redirect_uri),
        pct(IDENTITY_SCOPE),
        pct(challenge),
        pct(state),
        pct(nonce),
    )
}

/// Spawn the one-shot loopback listener. On the first request carrying both
/// `code` and `state` it replies with a friendly page, forwards them over `tx`,
/// and exits. Bounded by [`OAUTH_PENDING_TTL_SECS`] so it never leaks.
fn spawn_loopback_listener(listener: TcpListener, tx: oneshot::Sender<(String, String)>) {
    std::thread::spawn(move || {
        let _ = listener.set_nonblocking(true);
        let deadline = Instant::now() + Duration::from_secs(OAUTH_PENDING_TTL_SECS as u64);
        let mut tx = Some(tx);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _addr)) => {
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let first_line = req.lines().next().unwrap_or("");
                    if let Some((code, state)) = parse_callback_query(first_line) {
                        let _ = write_response(
                            &mut stream,
                            "200 OK",
                            "You're signed in to SeekerMail. You can close this tab and return to the app.",
                        );
                        if let Some(tx) = tx.take() {
                            let _ = tx.send((code, state));
                        }
                        return;
                    }
                    // Unrelated request (e.g. /favicon.ico): 404 and keep waiting.
                    let _ = write_response(&mut stream, "404 Not Found", "Not found.");
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(150));
                }
                Err(_) => break,
            }
        }
        // Deadline/error: drop `tx` → `complete` observes a closed channel.
    });
}

/// Write a minimal HTTP/1.1 response with a tiny HTML body.
fn write_response(stream: &mut TcpStream, status: &str, message: &str) -> std::io::Result<()> {
    let html = format!(
        "<!doctype html><html><body style=\"font-family:system-ui;padding:2rem\">{message}</body></html>"
    );
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
        html.len()
    );
    stream.write_all(resp.as_bytes())?;
    stream.flush()
}

/// Extract `code` + `state` from an HTTP request line
/// (`GET /?code=…&state=… HTTP/1.1`). Both must be present.
fn parse_callback_query(request_line: &str) -> Option<(String, String)> {
    let mut parts = request_line.split_whitespace();
    let _method = parts.next()?;
    let target = parts.next()?;
    let (_, query) = target.split_once('?')?;
    let mut code = None;
    let mut state = None;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            match k {
                "code" => code = Some(url_decode(v)),
                "state" => state = Some(url_decode(v)),
                _ => {}
            }
        }
    }
    Some((code?, state?))
}

/// Minimal RFC-3986 percent-encoding for query values (unreserved chars pass).
fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Percent-decode an `application/x-www-form-urlencoded` query value.
fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push((h << 4) | l);
                    i += 3;
                }
                _ => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// A URL-safe random nonce for the OIDC `nonce` claim (replay protection).
fn rand_nonce() -> String {
    use base64::Engine;
    use rand::RngCore;
    let mut bytes = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_has_identity_scope_and_pkce() {
        let url = build_authorize_url("cid123", "chal", "st8", "nonce1", "http://127.0.0.1:5500");
        assert!(url.contains("scope=openid%20email%20profile"));
        assert!(url.contains("code_challenge=chal"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=st8"));
        assert!(url.contains("nonce=nonce1"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A5500"));
        // HARD REDLINE: never a mail scope in the identity flow.
        assert!(!url.contains("mail.google.com"));
        assert!(!url.contains("gmail"));
    }

    #[test]
    fn parses_loopback_callback_line() {
        let (code, state) =
            parse_callback_query("GET /?code=4%2Fabc-DEF&state=xyz HTTP/1.1").unwrap();
        assert_eq!(code, "4/abc-DEF");
        assert_eq!(state, "xyz");
        // Missing a field → None.
        assert!(parse_callback_query("GET /?code=only HTTP/1.1").is_none());
        // Favicon / no query → None.
        assert!(parse_callback_query("GET /favicon.ico HTTP/1.1").is_none());
    }

    #[test]
    fn url_decode_handles_percent_and_plus() {
        assert_eq!(url_decode("a%2Fb+c"), "a/b c");
        assert_eq!(url_decode("plain"), "plain");
    }
}
