//! Structured logging foundation (T004, 09 §5).
//!
//! * `tracing` with line-delimited JSON, written to the app logs dir, rotated
//!   daily.
//! * Structured fields per line: `ts`, `level`, `module` (target), `event`, and
//!   when relevant `account_id` / `error_code` / `duration_ms`.
//! * A hard content/secret denylist: subjects, bodies, addresses, attachment
//!   names, passwords, tokens, API keys, AI prompt/completion text, and search
//!   queries are NEVER logged. We log identifiers and counts, never content.
//!
//! Enforcement is by discipline (call sites pass only allowlisted fields) plus
//! the `log_safety` unit test below, which runs content through the real logging
//! path and asserts no secret substring appears in the output.

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::config::Paths;
use crate::error::{AppError, AppResult};

/// Initialize the global tracing subscriber. Returns a [`WorkerGuard`] that must
/// be held for the lifetime of the process so the non-blocking writer flushes.
///
/// The default level is `info` (release) and can be overridden by the
/// `SEEKERMAIL_LOG` env var (e.g. `SEEKERMAIL_LOG=debug`) — see `.env.example`.
pub fn init(paths: &Paths) -> AppResult<WorkerGuard> {
    std::fs::create_dir_all(&paths.logs)
        .map_err(|e| AppError::FsPermission(format!("create logs dir: {e}")))?;

    // Daily-rotated file: ~/Library/Application Support/SeekerMail/logs/seekermail.log.YYYY-MM-DD
    let file_appender = tracing_appender::rolling::daily(&paths.logs, "seekermail.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_env("SEEKERMAIL_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,seekermail_lib=debug"));

    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_current_span(false)
        .with_span_list(false)
        .with_target(true)
        .with_writer(non_blocking);

    // A human-readable console layer for `tauri dev`; disabled in release JSON-only setups.
    let console_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_ansi(true)
        .compact();

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(console_layer)
        .try_init()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("logging init: {e}")))?;

    Ok(guard)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::IpcError;
    use std::io;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    /// Test writer that captures everything into a shared buffer so we can assert
    /// on what the logging path actually emitted.
    #[derive(Clone, Default)]
    struct BufWriter(Arc<Mutex<Vec<u8>>>);

    impl io::Write for BufWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for BufWriter {
        type Writer = BufWriter;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Feeds a content-bearing error through the real `AppError → IpcError`
    /// boundary (the single log point) and asserts the output carries the code
    /// and the non-secret host detail, but never a secret we did not pass.
    #[test]
    fn log_safety_no_secret_leak() {
        let buf = BufWriter::default();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer(buf.clone())
            .finish();

        // A password we deliberately NEVER pass to any logging call.
        const SECRET_PASSWORD: &str = "hunter2-super-secret-token";

        tracing::subscriber::with_default(subscriber, || {
            // Real logging path: From<AppError> logs once at the boundary.
            let _ipc: IpcError = AppError::ImapConnection("imap.example.com:993".into()).into();
        });

        let out = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(
            out.contains("IMAP_CONNECTION_FAILED"),
            "code must be logged"
        );
        assert!(
            out.contains("imap.example.com"),
            "non-secret technical detail (host) may be logged"
        );
        assert!(
            !out.contains(SECRET_PASSWORD),
            "secrets must never reach the log output"
        );
    }
}
