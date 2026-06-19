//! OAuth 2.0 (PKCE) initial grant for **Microsoft / Outlook only** (T015).
//!
//! Gmail no longer uses OAuth here: Google's `https://mail.google.com/` is a
//! *restricted* scope (annual paid CASA security assessment to ship publicly), so
//! Gmail mailbox import moves to IMAP + an App Password. Microsoft, by contrast,
//! retired App Passwords / basic auth for Outlook.com + Exchange Online, so OAuth
//! is the only path there — and Microsoft's OAuth verification is free. The split
//! is recorded in the knowledge base `docs/analysis/29_*`.
//!
//! Security rules enforced here:
//! * `code_verifier` and `state` are 32-byte CSPRNG values (see [`crate::account::pkce`]).
//! * `code_challenge = BASE64URL(SHA256(verifier))`, method `S256`.
//! * `complete` verifies the returned `state` against the pending nonce (CSRF).
//! * a pending grant older than [`OAUTH_PENDING_TTL_SECS`] is discarded.
//! * token strings are zeroized the instant they reach the Keychain — they never
//!   touch the DB, a log line, or an IPC return value (F_A2 §4).

use zeroize::Zeroize;

use serde::{Deserialize, Serialize};

use crate::account::pkce::new_pkce;
use crate::config::OAUTH_PENDING_TTL_SECS;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::types::Provider;
use crate::util::{now_unix, parse_uuid};

/// Deep-link redirect the provider sends the code back to (matches the registered
/// `seekermail://` scheme). Microsoft accepts a registered custom-scheme redirect
/// for a public/native client (unlike Google's Desktop-app client, which is why the
/// SeekerMail ID flow uses loopback). A follow-up migrates this to loopback too for
/// consistency + dev-testability (see `docs/analysis/29_*` §7).
pub const REDIRECT_URI: &str = "seekermail://oauth/callback";

/// Tauri event the lib.rs deep-link handler emits when it parses an account-mail
/// OAuth callback. The Add-Account wizard listens and forwards `code` + `state`
/// to `complete_oauth_flow` (where the CSRF check lives).
pub const MAIL_OAUTH_CALLBACK_EVENT: &str = "oauth:mail_callback";

/// Payload of [`MAIL_OAUTH_CALLBACK_EVENT`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailOAuthCallback {
    pub code: String,
    pub state: String,
}

/// Parse a `seekermail://oauth/callback?code=…&state=…` deep link. Mirrors
/// [`parse_recommended_callback`](crate::ai::recommended::parse_recommended_callback):
/// the lib.rs URL-scheme handler calls this to route a callback to the
/// account-mail flow (vs. the recommended flow on `/oauth/recommended`). Returns
/// `None` unless the URL is the mail callback and carries both `code` and `state`.
///
/// Both values are percent-decoded (M17): Microsoft authorization codes routinely
/// contain `/` (arriving as `%2F`); handing the encoded form to the token endpoint
/// re-encodes it to `%252F` and the exchange fails. The identity loopback path
/// decodes too — this keeps the two consistent.
pub fn parse_mail_callback(url: &str) -> Option<MailOAuthCallback> {
    let rest = url.strip_prefix(REDIRECT_URI)?;
    let query = rest.strip_prefix('?')?;
    let mut code = None;
    let mut state = None;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=')?;
        match k {
            "code" => code = Some(url_decode(v)),
            "state" => state = Some(url_decode(v)),
            _ => {}
        }
    }
    Some(MailOAuthCallback {
        code: code?,
        state: state?,
    })
}

/// Transient PKCE state for an in-flight grant, parked in `AppState.oauth`.
#[derive(Debug, Clone)]
pub struct PendingOAuth {
    pub verifier: String,
    pub state: String,
    pub provider: Provider,
    pub account_id: String,
    pub created_at: i64,
}

impl PendingOAuth {
    fn is_expired(&self, now: i64) -> bool {
        now - self.created_at > OAUTH_PENDING_TTL_SECS
    }
}

/// Provider authorization + token endpoints and the IMAP scope.
struct Endpoints {
    authorize_url: &'static str,
    token_url: &'static str,
    scope: &'static str,
}

fn endpoints(provider: Provider) -> AppResult<Endpoints> {
    match provider {
        Provider::Outlook => Ok(Endpoints {
            authorize_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
            scope: "https://outlook.office365.com/IMAP.AccessAsUser.All offline_access",
        }),
        // Gmail uses IMAP + App Password (not OAuth); IMAP/Exchange never reach here.
        _ => Err(AppError::Validation("provider does not use OAuth".into())),
    }
}

/// Env var holding a provider's OAuth client id. NOTE (deliberate deviation from
/// T015 §6): the spec suggested `env!` (compile-time), which would break every
/// build lacking the secret. We read at RUNTIME and surface a clean
/// `AUTH_OAUTH_FAILED` when unset, so the crate always compiles and dev builds
/// degrade gracefully.
fn client_id(provider: Provider) -> AppResult<String> {
    let var = match provider {
        Provider::Outlook => "SEEKERMAIL_MICROSOFT_CLIENT_ID",
        _ => return Err(AppError::Validation("provider does not use OAuth".into())),
    };
    std::env::var(var).map_err(|_| AppError::AuthOAuthFailed(format!("{var} is not configured")))
}

/// Build the provider authorization URL with all PKCE parameters.
fn build_authorize_url(
    provider: Provider,
    client_id: &str,
    challenge: &str,
    state: &str,
) -> AppResult<String> {
    let ep = endpoints(provider)?;
    let url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
        ep.authorize_url,
        pct(client_id),
        pct(REDIRECT_URI),
        pct(ep.scope),
        pct(challenge),
        pct(state),
    );
    Ok(url)
}

/// Begin a grant: park the PKCE state in `AppState` and return the authorize URL
/// for the caller to open in the system browser, plus the CSRF `state` nonce the
/// UI needs to complete the grant.
pub fn begin(
    state: &AppState,
    provider: Provider,
    account_id: &str,
) -> AppResult<(String, String)> {
    let cid = client_id(provider)?;
    let pkce = new_pkce();
    let url = build_authorize_url(provider, &cid, &pkce.challenge, &pkce.state)?;
    let state_nonce = pkce.state.clone();
    let pending = PendingOAuth {
        verifier: pkce.verifier,
        state: pkce.state,
        provider,
        account_id: account_id.to_string(),
        created_at: now_unix(),
    };
    *state.oauth.lock().expect("oauth mutex poisoned") = Some(pending);
    Ok((url, state_nonce))
}

/// Complete a grant from the deep-link callback: validate the `state` nonce + TTL,
/// exchange the code for tokens via the transport seam, store them in the
/// Keychain, and zeroize the plaintext. Returns the account id the grant was for
/// plus the access-token expiry (Unix s) so the caller can re-arm that account's
/// poll.
pub async fn complete(
    state: &AppState,
    code: &str,
    returned_state: &str,
) -> AppResult<(String, i64)> {
    // Take the pending grant out under the lock, then drop the guard before await.
    let pending = {
        let mut guard = state.oauth.lock().expect("oauth mutex poisoned");
        guard.take()
    };
    let pending =
        pending.ok_or_else(|| AppError::AuthOAuthFailed("no pending oauth grant".into()))?;

    if pending.is_expired(now_unix()) {
        return Err(AppError::AuthOAuthFailed("oauth grant expired".into()));
    }
    if pending.state != returned_state {
        return Err(AppError::AuthOAuthFailed("state mismatch".into()));
    }

    let ep = endpoints(pending.provider)?;
    let cid = client_id(pending.provider)?;
    let req = crate::net::TokenRequest {
        token_url: ep.token_url.to_string(),
        client_id: cid,
        redirect_uri: REDIRECT_URI.to_string(),
        code: Some(code.to_string()),
        code_verifier: Some(pending.verifier.clone()),
        refresh_token: None,
        scope: None,
    };

    let mut resp = state.net.oauth.exchange(req).await?;
    let expiry = now_unix() + resp.expires_in_secs;
    let uuid = parse_uuid(&pending.account_id)?;
    state.keychain.store_oauth(
        &uuid,
        &resp.access_token,
        resp.refresh_token.as_deref(),
        expiry,
    )?;

    // Scrub plaintext from memory after it reaches the Keychain (F_A2 §4).
    resp.access_token.zeroize();
    if let Some(rt) = resp.refresh_token.as_mut() {
        rt.zeroize();
    }
    Ok((pending.account_id, expiry))
}

/// Build a refresh-grant token request for a provider (T018). Shared with
/// `refresh.rs` so the endpoint/client-id resolution lives in one place.
pub(crate) fn refresh_token_request(
    provider: Provider,
    refresh_token: &str,
) -> AppResult<crate::net::TokenRequest> {
    let ep = endpoints(provider)?;
    let cid = client_id(provider)?;
    Ok(crate::net::TokenRequest {
        token_url: ep.token_url.to_string(),
        client_id: cid,
        redirect_uri: REDIRECT_URI.to_string(),
        code: None,
        code_verifier: None,
        refresh_token: Some(refresh_token.to_string()),
        scope: Some(ep.scope.to_string()),
    })
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

/// Percent-decode an `application/x-www-form-urlencoded` query value (M17). Mirrors
/// the identity loopback decoder so both OAuth paths handle `%2F`-bearing codes
/// identically.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_has_required_params() {
        let url = build_authorize_url(Provider::Outlook, "cid123", "chal", "nonce").unwrap();
        assert!(url.contains("code_challenge=chal"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=nonce"));
        assert!(url.contains("redirect_uri=seekermail%3A%2F%2Foauth%2Fcallback"));
        assert!(url.contains("login.microsoftonline.com"));
        // offline_access (refresh token) rides in the scope for Microsoft.
        assert!(url.contains("offline_access"));
    }

    #[test]
    fn non_oauth_provider_rejected() {
        assert!(endpoints(Provider::Imap).is_err());
        assert!(client_id(Provider::Imap).is_err());
        // Gmail no longer uses OAuth — it imports via IMAP + App Password.
        assert!(endpoints(Provider::Gmail).is_err());
        assert!(client_id(Provider::Gmail).is_err());
    }

    #[test]
    fn mail_callback_parse_decodes_code_and_state() {
        // Microsoft codes contain `/` → arrive as `%2F`; must be decoded (M17).
        let cb =
            parse_mail_callback("seekermail://oauth/callback?code=M%2Fabc-DEF&state=xyz").unwrap();
        assert_eq!(cb.code, "M/abc-DEF");
        assert_eq!(cb.state, "xyz");
        // The recommended-flow path is NOT ours.
        assert!(parse_mail_callback("seekermail://oauth/recommended?code=a&state=b").is_none());
        // Missing a field → None.
        assert!(parse_mail_callback("seekermail://oauth/callback?code=a").is_none());
        // Not our scheme/path → None.
        assert!(parse_mail_callback("https://example.com/?code=a&state=b").is_none());
    }
}
