//! Google OIDC `id_token` verification against Google's JWKS (T121).
//!
//! [`fetch_and_verify`] fetches Google's published RSA public keys, selects the
//! one matching the token's `kid`, and delegates to [`verify_id_token_with_key`]
//! — the pure, unit-testable core (validated below against a mock IdP key).

use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;

use crate::error::{AppError, AppResult};

/// Google's JWKS (RSA public keys) endpoint.
const GOOGLE_JWKS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";
/// Accepted `iss` values for a Google-issued `id_token`.
const GOOGLE_ISSUERS: [&str; 2] = ["https://accounts.google.com", "accounts.google.com"];

/// The identity claims SeekerMail consumes from a verified Google `id_token`.
/// The registered claims (`aud`/`iss`/`exp`) are kept so they always deserialize;
/// signature, audience, issuer and expiry are enforced by [`jsonwebtoken`].
#[derive(Debug, Clone, Deserialize)]
pub struct GoogleIdClaims {
    pub sub: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default, deserialize_with = "de_bool_lenient")]
    pub email_verified: Option<bool>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub nonce: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub aud: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub iss: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub exp: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct Jwk {
    kid: String,
    n: String,
    e: String,
}

#[derive(Debug, Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

/// Fetch Google's JWKS and fully verify `id_token` (signature + `iss`/`aud`/`exp`
/// + `nonce`). Returns the identity claims on success.
pub async fn fetch_and_verify(
    id_token: &str,
    client_id: &str,
    expected_nonce: Option<&str>,
) -> AppResult<GoogleIdClaims> {
    let header = decode_header(id_token)
        .map_err(|e| AppError::AuthOAuthFailed(format!("id_token header: {e}")))?;
    let kid = header
        .kid
        .ok_or_else(|| AppError::AuthOAuthFailed("id_token has no kid".into()))?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| AppError::AuthOAuthFailed(format!("http client: {e}")))?;
    let jwks: Jwks = client
        .get(GOOGLE_JWKS_URL)
        .send()
        .await
        .map_err(|e| AppError::AuthOAuthFailed(format!("jwks fetch failed: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::AuthOAuthFailed(format!("jwks decode failed: {e}")))?;
    let jwk = jwks
        .keys
        .iter()
        .find(|k| k.kid == kid)
        .ok_or_else(|| AppError::AuthOAuthFailed("no matching JWKS key".into()))?;
    let key = DecodingKey::from_rsa_components(&jwk.n, &jwk.e)
        .map_err(|e| AppError::AuthOAuthFailed(format!("jwks key: {e}")))?;

    verify_id_token_with_key(id_token, &key, client_id, expected_nonce)
}

/// Verify `id_token` against a known decoding key: RS256 signature, audience ==
/// `client_id`, a Google issuer, a present + future `exp`, and (if given) a
/// matching `nonce`. The pure, network-free core ("JWKS verification against a
/// mock IdP", T121 §7).
pub fn verify_id_token_with_key(
    id_token: &str,
    key: &DecodingKey,
    client_id: &str,
    expected_nonce: Option<&str>,
) -> AppResult<GoogleIdClaims> {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[client_id]);
    validation.set_issuer(&GOOGLE_ISSUERS);
    validation.set_required_spec_claims(&["exp", "aud", "iss"]);
    let data = decode::<GoogleIdClaims>(id_token, key, &validation)
        .map_err(|e| AppError::AuthOAuthFailed(format!("id_token verification failed: {e}")))?;
    let claims = data.claims;
    if let Some(expected) = expected_nonce {
        if claims.nonce.as_deref() != Some(expected) {
            return Err(AppError::AuthOAuthFailed("id_token nonce mismatch".into()));
        }
    }
    Ok(claims)
}

/// Google's `email_verified` is normally a JSON bool, but some legacy tokens send
/// the string `"true"`. Accept either.
fn de_bool_lenient<'de, D>(d: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum BoolOrStr {
        Bool(bool),
        Str(String),
    }
    let v = Option::<BoolOrStr>::deserialize(d)?;
    Ok(match v {
        Some(BoolOrStr::Bool(b)) => Some(b),
        Some(BoolOrStr::Str(s)) => Some(s == "true"),
        None => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde::Serialize;
    use std::time::{SystemTime, UNIX_EPOCH};

    // Throwaway 2048-bit RSA keypair for the mock IdP — TEST ONLY, never used in
    // production. Generated with `openssl genrsa 2048`.
    const TEST_PRIV: &str = "-----BEGIN PRIVATE KEY-----
MIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQCgvCAw1CcboRzq
orMcVDvGhj+lYx0gcLYvMnIynsGc1COutXiqLpO0sX4ZtapgsoSie6eszFXUl7iZ
yFXSemgPpXXq5glKj4s+U6sOxaCnqMOk5u31I7L1l1bqy0ldj8b55b314xlPN8YA
OlLWS/2apbeniu4mR2ng4hLovp3TSCSS48SMebrVEsCTOJjRS+B8ExzGM4nE+s8J
42twUba1/sFYHFutcQ0tLcsKMTp8NT/UIwD8TT35KldKBBGXanLRjIaPAWWmTKak
3ka81Gxiw51f9rVJdBuhKudTMLkXnJOVMOYPDQpqRXVtI1n2JIS/dF5QlXcIpbSf
BLUVotWVAgMBAAECggEABHmweMOTci6+eB7/FzEWN/0ZbRCxKSvSOsawDM5ETEpQ
0wa8/RoOZUfS38Lh407dKAwC22oWErUFwAxmrVVYq+Tav2dXx2JvSVU5jE/+3OQz
TFZctkhf7XwhAKkX2hnMe71EmIHR/NVr9yooj8xuW321Ox5AasLuxQLubVoPWWhb
w+QCXZq+6h3B+5ycuO8PqCitHCz4kwG277vHiGna5Z9GZQiicHSZvUfFb24QiHwq
R0QRxQiB5rhAOq6dlhECUD8DnR68ko/1WSMJfRBYhHvRGl6M1gqP37mJ7+oLJ4OQ
A3DU3CCEDCa8UVz6DXO1nTKGVmRgCAM7uLOsAoQ7SQKBgQDCW4Hu+ADN0G8LdTkx
G1BCReQdPRUKIYmnsuE0Mm6gD1l3CdvszYXwW8RVynqK35m1EyHQZBnaBczPse44
mUcxkZPx+gRYohtD8XpNXjRKb2uPPgHlGQiQ428H/fqeU44TYCuTzw5PBEfGPyQI
rnSL63tKNcTQ3W3EuOXHOEDwyQKBgQDTtrHe5L1Q7R2/qFA8vVf0CkuUjDqykWis
hT1zyUi48DSP8/gLrL2qUgY0lKBHgdqaFzN63YlvNl572psonl/O6QNwDUabjCH3
QgPAHahu/tYPuEtGcYfJsUuLg6auIsJaU9Y+7i+9cbxh1EYJfrc5D3ATzP4dZhHH
kt8r2RHQbQKBgAvGxam4JzxRS9ky4iNCl2tclsTaxaKWg6PAp/qkr6VNKMuYslW3
4ky9ErlsCl7Ny594KE1bM2HNhipzio6tYu3y9zbrQkYolGRahmGXuq1j8O1+AVlj
WeyFi129mujrASnVYu6S1jgdd0fg3YsVHwS3YQIPHfzV3efUmD+o/e5ZAoGAB/uA
c13+gVmfYIWRGOkussXcmao74FW5M6AGdCInuslbwf254X7O2+gh0cO0011jB6JO
T5igwO+02kigxwRJqnyAo63sdprvAOqdR5YWrrCvE4KoW+yV6RXlOkppc3FeEJfO
oSrL5AGwz6N4TI1ZjS421JhLEIKzsumnvnh9wnUCgYBj6RM1PEKnd9BVPhh/lh+0
2fa183mt3UZyt02FAFsqXSHlIXuWBfZfvDQgYtnWdYFmvZ4UUtAW49UhZydcj6U1
1pi4k6QzDhWY/y16t8tBbL8IRBQ8qDzYT5KBsNR9bOj+Sc610DNn476C8Hx3FqpN
+MhqqBpxQa0PHPlbOnGAqQ==
-----END PRIVATE KEY-----
";
    const TEST_PUB: &str = "-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAoLwgMNQnG6Ec6qKzHFQ7
xoY/pWMdIHC2LzJyMp7BnNQjrrV4qi6TtLF+GbWqYLKEonunrMxV1Je4mchV0npo
D6V16uYJSo+LPlOrDsWgp6jDpObt9SOy9ZdW6stJXY/G+eW99eMZTzfGADpS1kv9
mqW3p4ruJkdp4OIS6L6d00gkkuPEjHm61RLAkziY0UvgfBMcxjOJxPrPCeNrcFG2
tf7BWBxbrXENLS3LCjE6fDU/1CMA/E09+SpXSgQRl2py0YyGjwFlpkympN5GvNRs
YsOdX/a1SXQboSrnUzC5F5yTlTDmDw0KakV1bSNZ9iSEv3ReUJV3CKW0nwS1FaLV
lQIDAQAB
-----END PUBLIC KEY-----
";

    #[derive(Serialize)]
    struct TestClaims<'a> {
        sub: &'a str,
        email: &'a str,
        email_verified: bool,
        name: &'a str,
        aud: &'a str,
        iss: &'a str,
        exp: usize,
        nonce: &'a str,
    }

    fn now() -> usize {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize
    }

    fn make_token(aud: &str, iss: &str, exp: usize, nonce: &str) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-kid".into());
        let claims = TestClaims {
            sub: "sub-123",
            email: "victor@example.com",
            email_verified: true,
            name: "Victor",
            aud,
            iss,
            exp,
            nonce,
        };
        let key = EncodingKey::from_rsa_pem(TEST_PRIV.as_bytes()).expect("encoding key");
        encode(&header, &claims, &key).expect("encode token")
    }

    fn pub_key() -> DecodingKey {
        DecodingKey::from_rsa_pem(TEST_PUB.as_bytes()).expect("decoding key")
    }

    #[test]
    fn verifies_a_valid_token() {
        let token = make_token(
            "client-123",
            "https://accounts.google.com",
            now() + 3600,
            "nonce-abc",
        );
        let claims =
            verify_id_token_with_key(&token, &pub_key(), "client-123", Some("nonce-abc")).unwrap();
        assert_eq!(claims.sub, "sub-123");
        assert_eq!(claims.email.as_deref(), Some("victor@example.com"));
        assert_eq!(claims.email_verified, Some(true));
        assert_eq!(claims.name.as_deref(), Some("Victor"));
    }

    #[test]
    fn rejects_wrong_audience() {
        let token = make_token(
            "client-123",
            "https://accounts.google.com",
            now() + 3600,
            "n",
        );
        assert!(verify_id_token_with_key(&token, &pub_key(), "other-client", Some("n")).is_err());
    }

    #[test]
    fn rejects_bad_issuer() {
        let token = make_token("client-123", "https://evil.example.com", now() + 3600, "n");
        assert!(verify_id_token_with_key(&token, &pub_key(), "client-123", Some("n")).is_err());
    }

    #[test]
    fn rejects_expired_token() {
        // Expired well beyond the validator's default 60 s clock-skew leeway.
        let token = make_token(
            "client-123",
            "https://accounts.google.com",
            now() - 7200,
            "n",
        );
        assert!(verify_id_token_with_key(&token, &pub_key(), "client-123", Some("n")).is_err());
    }

    #[test]
    fn rejects_nonce_mismatch() {
        let token = make_token(
            "client-123",
            "https://accounts.google.com",
            now() + 3600,
            "real-nonce",
        );
        assert!(verify_id_token_with_key(&token, &pub_key(), "client-123", Some("WRONG")).is_err());
    }
}
