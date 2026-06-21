//! Google OIDC `id_token` verification.
//!
//! Verifies the `id_token` returned by Google's OIDC flow:
//!   1. Fetches Google's public JWKS (cached in memory with a 5-minute TTL).
//!   2. Decodes the JWT header to find the matching `kid`.
//!   3. Verifies the RS256 signature, `aud`, `iss`, and `exp`.
//!   4. Returns the extracted claims.
//!
//! Scope guard: ONLY `openid email profile` claims are extracted.
//! Mail scopes are NEVER requested by this service.

use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::error::{AppError, AppResult};

const GOOGLE_JWKS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";
const GOOGLE_ISSUER: &str = "https://accounts.google.com";
const JWKS_CACHE_TTL: Duration = Duration::from_secs(300); // 5 minutes

// ── Types ────────────────────────────────────────────────────────────────────

/// Claims extracted from a verified Google id_token.
/// Only fields from scope `openid email profile` are mapped here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcClaims {
    pub sub: String,
    pub email: String,
    pub email_verified: bool,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JwksKey {
    kid: String,
    n: String,
    e: String,
}

#[derive(Debug, Deserialize)]
struct Jwks {
    keys: Vec<JwksKey>,
}

// ── JWKS cache ───────────────────────────────────────────────────────────────

struct JwksCache {
    data: Jwks,
    fetched_at: Instant,
}

#[derive(Clone)]
pub struct OidcVerifier {
    audience: String,
    http: Client,
    cache: Arc<Mutex<Option<JwksCache>>>,
}

impl OidcVerifier {
    pub fn new(audience: impl Into<String>) -> Self {
        Self {
            audience: audience.into(),
            http: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("failed to build HTTP client"),
            cache: Arc::new(Mutex::new(None)),
        }
    }

    /// Fetch the JWKS, returning the cached copy if it's still fresh.
    async fn get_jwks(&self) -> AppResult<Jwks> {
        let mut guard = self.cache.lock().await;

        if let Some(ref cached) = *guard {
            if cached.fetched_at.elapsed() < JWKS_CACHE_TTL {
                // Reconstruct a Jwks from the cached keys to avoid Clone on the whole struct.
                // We fetch fresh below on cache miss — here we just signal "use cache".
                // (The locked scope is short so this is fine.)
                let keys: Vec<JwksKey> = cached
                    .data
                    .keys
                    .iter()
                    .map(|k| JwksKey {
                        kid: k.kid.clone(),
                        n: k.n.clone(),
                        e: k.e.clone(),
                    })
                    .collect();
                return Ok(Jwks { keys });
            }
        }

        // Cache miss or stale — fetch from Google.
        let jwks: Jwks = self
            .http
            .get(GOOGLE_JWKS_URL)
            .send()
            .await
            .map_err(|e| AppError::OidcVerify(format!("JWKS fetch failed: {e}")))?
            .json()
            .await
            .map_err(|e| AppError::OidcVerify(format!("JWKS parse failed: {e}")))?;

        *guard = Some(JwksCache {
            data: Jwks {
                keys: jwks
                    .keys
                    .iter()
                    .map(|k| JwksKey {
                        kid: k.kid.clone(),
                        n: k.n.clone(),
                        e: k.e.clone(),
                    })
                    .collect(),
            },
            fetched_at: Instant::now(),
        });

        Ok(jwks)
    }

    /// Verify an `id_token` and return the extracted claims.
    pub async fn verify(&self, id_token: &str) -> AppResult<OidcClaims> {
        // Step 1: decode the header to find the key ID.
        let header = decode_header(id_token)
            .map_err(|e| AppError::OidcVerify(format!("bad JWT header: {e}")))?;
        let kid = header
            .kid
            .ok_or_else(|| AppError::OidcVerify("missing kid in JWT header".into()))?;

        // Step 2: find the matching public key in the JWKS.
        let jwks = self.get_jwks().await?;
        let key = jwks
            .keys
            .iter()
            .find(|k| k.kid == kid)
            .ok_or_else(|| AppError::OidcVerify(format!("no key with kid={kid} in JWKS")))?;

        let decoding_key = DecodingKey::from_rsa_components(&key.n, &key.e)
            .map_err(|e| AppError::OidcVerify(format!("bad RSA key: {e}")))?;

        // Step 3: validate signature, aud, iss, exp.
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&self.audience]);
        validation.set_issuer(&[GOOGLE_ISSUER, "accounts.google.com"]);

        let token_data = decode::<serde_json::Value>(id_token, &decoding_key, &validation)
            .map_err(|e| AppError::OidcVerify(format!("JWT validation failed: {e}")))?;

        // Step 4: extract only the claims we care about.
        let claims = &token_data.claims;
        let sub = claims["sub"]
            .as_str()
            .ok_or_else(|| AppError::OidcVerify("missing sub claim".into()))?
            .to_string();
        let email = claims["email"]
            .as_str()
            .ok_or_else(|| AppError::OidcVerify("missing email claim".into()))?
            .to_string();
        let email_verified = claims["email_verified"].as_bool().unwrap_or(false);
        let name = claims["name"].as_str().map(ToString::to_string);

        Ok(OidcClaims {
            sub,
            email,
            email_verified,
            name,
        })
    }
}
