//! Shared PKCE primitives (RFC 7636) for every OAuth/OIDC grant in the app.
//!
//! Extracted from `account::oauth` so the three callers — the SeekerMail ID
//! identity sign-in ([`crate::identity`]), the mailbox OAuth grant
//! ([`crate::account::oauth`], Outlook only), and the recommended-AI-provider
//! grant ([`crate::ai::recommended`]) — share one implementation instead of each
//! owning a copy. Pure crypto: a 32-byte CSPRNG verifier, its S256 challenge, and
//! a 32-byte CSPRNG `state` nonce. No I/O, no provider knowledge.

use base64::Engine;
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};

/// One generated PKCE challenge: the secret `verifier`, its S256 `challenge`, and
/// a CSRF `state` nonce.
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

/// A URL-safe, unpadded base64 string of `n` CSPRNG bytes.
fn random_b64url(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes)
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
}
