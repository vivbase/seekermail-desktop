use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Config {
    /// Postgres connection URL (from DATABASE_URL env var)
    pub database_url: String,

    /// Google OAuth Client ID — used to verify the `aud` claim in id_tokens.
    /// Set GOOGLE_OIDC_AUDIENCE=<client-id>.apps.googleusercontent.com
    pub google_oidc_audience: String,

    /// Secret used to sign session tokens (reserved for future HMAC MACs — T121b).
    /// Generate with: openssl rand -hex 32
    #[allow(dead_code)]
    pub session_token_secret: String,

    /// How long a session token is valid (seconds). Default: 30 days.
    pub session_ttl_secs: i64,

    /// TCP port to bind on. Default: 8080.
    pub port: u16,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        // Load .env if present (dev only; production uses platform env vars)
        let _ = dotenvy::dotenv();

        Ok(Config {
            database_url: std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?,
            google_oidc_audience: std::env::var("GOOGLE_OIDC_AUDIENCE")
                .context("GOOGLE_OIDC_AUDIENCE must be set (your Google Client ID)")?,
            session_token_secret: std::env::var("SESSION_TOKEN_SECRET")
                .context("SESSION_TOKEN_SECRET must be set (run: openssl rand -hex 32)")?,
            session_ttl_secs: std::env::var("SESSION_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30 * 24 * 3600), // 30 days
            port: std::env::var("PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8080),
        })
    }
}
