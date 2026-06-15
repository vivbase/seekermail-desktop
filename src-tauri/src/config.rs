//! Application paths (03 §3). All app data lives under one local-first root:
//! the per-OS data dir from `directories` joined with `SeekerMail` — macOS
//! Application Support, Windows `%APPDATA%`, Linux XDG data dir (T116).

use std::path::PathBuf;

use crate::error::{AppError, AppResult};

// ── Tunable constants (no magic numbers inline, T014 §9) ─────────────────────

/// Per-protocol connection-probe timeout (T014).
pub const CONNECTION_TEST_TIMEOUT_SECS: u64 = 15;
/// Mailbox-sampling total timeout before degrading to "can't estimate" (T016).
pub const SAMPLING_TIMEOUT_SECS: u64 = 10;
/// A transient OAuth grant is discarded if not completed within this window (T015).
pub const OAUTH_PENDING_TTL_SECS: i64 = 300;
/// Refresh an access token when it has less than this much life left (T018).
pub const TOKEN_REFRESH_LEEWAY_SECS: i64 = 300;
/// Global cap on concurrent per-account poll tasks (T021, F_A4 §7).
pub const MAX_CONCURRENT_POLLS: usize = 4;
/// Global cap on concurrently back-filling accounts (T022, F_A4 §4.3).
pub const MAX_CONCURRENT_BACKFILLS: usize = 2;
/// Batch size for history backfill FETCH (T022, F_A4 §3).
pub const BACKFILL_BATCH_SIZE: usize = 200;
/// Pause between backfill batches, milliseconds (T022, server-friendly).
pub const BACKFILL_BATCH_PAUSE_MS: u64 = 500;
/// Batch size for incremental poll FETCH (T022).
pub const POLL_FETCH_BATCH_SIZE: usize = 50;
/// Concurrent manual attachment downloads (T025, F_A5 §5.3).
pub const ATTACH_MANUAL_CONCURRENCY: usize = 2;
/// Concurrent auto attachment downloads per account (T025).
pub const ATTACH_AUTO_PER_ACCOUNT: usize = 2;
/// Global concurrent auto attachment downloads (T025).
pub const ATTACH_AUTO_GLOBAL: usize = 4;

/// Resolved on-disk locations for app data. The SQLite DB is the source of
/// truth; `vectors/` (LanceDB) is derived; `attachments/` and `logs/` are local.
#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub db: PathBuf,
    pub vectors: PathBuf,
    pub attachments: PathBuf,
    pub logs: PathBuf,
    /// Bundled model resources (bge-m3 ONNX + tokenizer + lock, T010/T030). In a
    /// packaged app this is Tauri's `resource_dir()`; in dev it is overridable via
    /// `SEEKERMAIL_RESOURCE_DIR`, else `{root}/models`.
    pub models: PathBuf,
}

impl Paths {
    /// Resolve the app data root and derive the subpaths. Honors the optional
    /// `SEEKERMAIL_DATA_DIR` dev override (documented in `.env.example`); otherwise
    /// uses the per-OS data dir (`directories::BaseDirs::data_dir()`): macOS
    /// Application Support, Windows `%APPDATA%`, Linux XDG data dir (T116).
    pub fn resolve() -> AppResult<Self> {
        let root = match std::env::var_os("SEEKERMAIL_DATA_DIR") {
            Some(dir) if !dir.is_empty() => std::path::PathBuf::from(dir),
            _ => {
                let base = directories::BaseDirs::new()
                    .ok_or_else(|| AppError::Internal(anyhow::anyhow!("no home directory")))?;
                base.data_dir().join("SeekerMail")
            }
        };
        let models = match std::env::var_os("SEEKERMAIL_RESOURCE_DIR") {
            Some(dir) if !dir.is_empty() => std::path::PathBuf::from(dir),
            _ => root.join("models"),
        };
        Ok(Self {
            db: root.join("seekermail.db"),
            vectors: root.join("vectors"),
            attachments: root.join("attachments"),
            logs: root.join("logs"),
            models,
            root,
        })
    }

    /// Create every app directory if missing. Safe to call on every startup.
    pub fn ensure_dirs(&self) -> AppResult<()> {
        for dir in [&self.root, &self.vectors, &self.attachments, &self.logs] {
            std::fs::create_dir_all(dir)
                .map_err(|e| AppError::FsPermission(format!("create {}: {e}", dir.display())))?;
        }
        Ok(())
    }

    /// The bge-m3 ONNX model file (T010/T030).
    pub fn model_onnx(&self) -> PathBuf {
        self.models.join("bge-m3.onnx")
    }

    /// The bge-m3 tokenizer JSON (T010/T030).
    pub fn model_tokenizer(&self) -> PathBuf {
        self.models.join("tokenizer.json")
    }

    /// The model checksum lock written by T010 (`{ "sha256": "…" }`, T030 §6).
    pub fn model_lock(&self) -> PathBuf {
        self.models.join("model.lock.json")
    }

    /// An account's private data root: `{root}/{accountUUID}/` (F_A3 §4.4). Blob
    /// files are isolated per account here so a single account can be wiped by
    /// removing one directory (T020/T026).
    pub fn account_dir(&self, account_id: &str) -> std::path::PathBuf {
        self.root.join(account_id)
    }

    /// An account's attachment tree: `{root}/{accountUUID}/attachments/`.
    pub fn account_attachments_dir(&self, account_id: &str) -> std::path::PathBuf {
        self.account_dir(account_id).join("attachments")
    }
}

#[cfg(test)]
mod tests {
    use super::Paths;

    // Per-OS data-dir resolution (T116). Skipped when the dev override is set, so
    // the assertions describe the real OS default, not the override.
    #[cfg(target_os = "macos")]
    #[test]
    fn paths_macos_data_dir() {
        if std::env::var_os("SEEKERMAIL_DATA_DIR").is_some() {
            return;
        }
        let paths = Paths::resolve().expect("resolve");
        let root = paths.root.to_string_lossy();
        assert!(root.contains("Application Support"), "macOS root: {root}");
        assert!(paths.root.ends_with("SeekerMail"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn paths_windows_data_dir() {
        if std::env::var_os("SEEKERMAIL_DATA_DIR").is_some() {
            return;
        }
        let paths = Paths::resolve().expect("resolve");
        let root = paths.root.to_string_lossy();
        assert!(root.contains("AppData"), "windows root: {root}");
        assert!(paths.root.ends_with("SeekerMail"));
    }
}
