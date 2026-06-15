//! OAuth 2.0 (PKCE) initial grant for Google + Microsoft (T015).
//!
//! Security rules enforced here:
//! * `code_verifier` and `state` are 32-byte CSPRNG values (`rand::rngs::OsRng`).
//! * `code_challenge = BASE64URL(SHA256(verifier))`, method `S256`.
//! * `complete` verifies the returned `state` against the pending nonce (CSRF).
//! * a pending grant older than [`OAUTH_PENDING_TTL_SECS`] is discarded.
//! * token strings are zeroized the instant they reach the Keychain — they never
//!   touch the DB, a log line, or an IPC return value (F_A2 §4).

use base64::Engine;
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::config::OAUTH_PENDING_TTL_SECS;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::types::Provider;
use crate::util::{now_unix, parse_uuid};

/// Deep-link redirect the provider sends the code back to (matches the registered
/// `seekermail://` scheme).
pub const REDIRECT_URI: &str = "seekermail://oauth/callback";

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
        Provider::Gmail => Ok(Endpoints {
            authorize_url: "https://accounts.google.com/o/oauth2/v2/auth",
            token_url: "https://oauth2.googleapis.com/token",
            scope: "https://mail.google.com/",
        }),
        Provider::Outlook => Ok(Endpoints {
            authorize_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
            scope: "https://outlook.office365.com/IMAP.AccessAsUser.All offline_access",
        }),
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
        Provider::Gmail => "SEEKERMAIL_GOOGLE_CLIENT_ID",
        Provider::Outlook => "SEEKERMAIL_MICROSOFT_CLIENT_ID",
        _ => return Err(AppError::Validation("provider does not use OAuth".into())),
    };
    std::env::var(var).map_err(|_| AppError::AuthOAuthFailed(format!("{var} is not configured")))
}

/// One generated PKCE challenge.
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
    pub state: String,
}

/// Generate a fresh verifier + S256 challenge + state nonce.
pub fn new_pkce() -> Pkce {
    let verifier = random_b64url(32);
    let state = random_b64url(32);
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize());
    Pkce {
        verifier,
        challenge,
        state,
    }
}

fn random_b64url(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes)
}

/// Build the provider authorization URL with all PKCE parameters.
fn build_authorize_url(
    provider: Provider,
    client_id: &str,
    challenge: &str,
    state: &str,
) -> AppResult<String> {
    let ep = endpoints(provider)?;
    let mut url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
        ep.authorize_url,
        pct(client_id),
        pct(REDIRECT_URI),
        pct(ep.scope),
        pct(challenge),
        pct(state),
    );
    // Ask Google for a refresh token explicitly.
    if matches!(provider, Provider::Gmail) {
        url.push_str("&access_type=offline&prompt=consent");
    }
    Ok(url)
}

/// Begin a grant: park the PKCE state in `AppState` and return the authorize URL
/// for the caller to open in the system browser.
pub fn begin(state: &AppState, provider: Provider, account_id: &str) -> AppResult<String> {
    let cid = client_id(provider)?;
    let pkce = new_pkce();
    let url = build_authorize_url(provider, &cid, &pkce.challenge, &pkce.state)?;
    let pending = PendingOAuth {
        verifier: pkce.verifier,
        state: pkce.state,
        provider,
        account_id: account_id.to_string(),
        created_at: now_unix(),
    };
    *state.oauth.lock().expect("oauth mutex poisoned") = Some(pending);
    Ok(url)
}

/// Complete a grant from the deep-link callback: validate the `state` nonce + TTL,
/// exchange the code for tokens via the transport seam, store them in the
/// Keychain, and zeroize the plaintext. Returns the access-token expiry (Unix s).
pub async fn complete(state: &AppState, code: &str, returned_state: &str) -> AppResult<i64> {
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
    Ok(expiry)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_s256_of_verifier() {
        let p = new_pkce();
        let mut hasher = Sha256::new();
        hasher.update(p.verifier.as_bytes());
        let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize());
        assert_eq!(p.challenge, expected);
        // Verifier and state are distinct, URL-safe, no padding.
        assert_ne!(p.verifier, p.state);
        assert!(!p.challenge.contains('='));
        assert!(!p.challenge.contains('+'));
    }

    #[test]
    fn authorize_url_has_required_params() {
        let url = build_authorize_url(Provider::Gmail, "cid123", "chal", "nonce").unwrap();
        assert!(url.contains("code_challenge=chal"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=nonce"));
        assert!(url.contains("redirect_uri=seekermail%3A%2F%2Foauth%2Fcallback"));
        assert!(url.contains("access_type=offline")); // google refresh token
    }

    #[test]
    fn non_oauth_provider_rejected() {
        assert!(endpoints(Provider::Imap).is_err());
        assert!(client_id(Provider::Imap).is_err());
    }
}
