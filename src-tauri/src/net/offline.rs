//! Offline transport implementations — the default build (no network crates).
//!
//! They let the app boot and the service/command layers run end-to-end without a
//! server: probes report failure with a clear message, the IMAP factory refuses
//! to connect, and the token endpoint reports an OAuth failure. Unit tests inject
//! their own fakes instead of using these.

use futures::future::BoxFuture;

use super::{
    offline_err, ConnProbe, ConnProbeConfig, ConnProbeReport, ImapCreds, ImapFactory, ImapSession,
    TokenEndpoint, TokenRequest, TokenResponse,
};
use crate::error::{AppError, AppResult};

/// IMAP factory that always reports the build is offline.
pub struct OfflineImapFactory;

impl ImapFactory for OfflineImapFactory {
    fn open(&self, _creds: ImapCreds) -> BoxFuture<'_, AppResult<Box<dyn ImapSession>>> {
        Box::pin(async { Err(offline_err("imap connect")) })
    }
}

/// Connection probe that always fails in-band (never an `Err`).
pub struct OfflineConnProbe;

impl ConnProbe for OfflineConnProbe {
    fn verify(&self, _cfg: ConnProbeConfig) -> BoxFuture<'_, ConnProbeReport> {
        Box::pin(async {
            ConnProbeReport {
                imap_ok: false,
                smtp_ok: false,
                error_message: Some(
                    "Connection testing requires a networked build (--features live-net).".into(),
                ),
            }
        })
    }
}

/// OAuth token endpoint that reports the flow can't run offline.
pub struct OfflineTokenEndpoint;

impl TokenEndpoint for OfflineTokenEndpoint {
    fn exchange(&self, _req: TokenRequest) -> BoxFuture<'_, AppResult<TokenResponse>> {
        Box::pin(async {
            Err(AppError::AuthOAuthFailed(
                "offline build (enable --features live-net)".into(),
            ))
        })
    }
}
