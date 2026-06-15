//! `DiskBlobStore` — attachment files on disk + disk accounting (T020, A5).
//!
//! Layout (relative to `Paths.root`, F_A5 §5.1):
//! `{accountUUID}/attachments/{YYYY}/{MM}/{mailUUID}/{filename}`. Only the
//! relative path is stored in `attachments.local_path`, so moving the data root
//! is a one-line change. Bytes never live in the DB.
//!
//! Hard rules enforced here:
//! * executable attachments are NEVER written (F_A5 §7);
//! * a write is refused when free space would drop below the 500 MB low-watermark
//!   or below `needed × 1.5` (F_A3 §6).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use super::Db;
use crate::error::{AppError, AppResult};
use crate::types::DiskUsage;
use crate::util::now_unix;

/// Extensions that must never be written or opened (F_A5 §7).
pub const BLOCKED_EXTS: &[&str] = &[
    ".exe", ".bat", ".scr", ".cmd", ".msi", ".app", ".sh", ".ps1", ".vbs", ".jar", ".com", ".dll",
];

/// 500 MB free-space floor (F_A3 §6).
pub const LOW_WATERMARK_BYTES: u64 = 500 * 1024 * 1024;

/// Streaming-write buffer (F_A5 §5.2).
const CHUNK: usize = 64 * 1024;

/// True if `filename`'s extension is on the executable block-list.
pub fn is_blocked_executable(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    BLOCKED_EXTS.iter().any(|ext| lower.ends_with(ext))
}

/// Hex SHA-256 of a byte slice (dedup key, F_A5 §5.4).
pub fn sha256_of(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex(&h.finalize())
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Result of a streaming attachment write.
#[derive(Debug, Clone)]
pub struct WriteResult {
    pub relative_path: String,
    pub sha256: String,
    pub bytes_written: u64,
}

/// Free-space probe seam so the low-watermark guard is unit-testable (T020 §8).
pub trait DiskSpaceProbe: Send + Sync {
    /// Bytes available on the volume backing `dir`.
    fn available_bytes(&self, dir: &Path) -> u64;
}

/// Real probe (sysinfo). On any failure it reports "plenty" so a transient probe
/// error never blocks a legitimate write.
pub struct SysinfoDiskSpace;

impl DiskSpaceProbe for SysinfoDiskSpace {
    fn available_bytes(&self, dir: &Path) -> u64 {
        use sysinfo::Disks;
        let disks = Disks::new_with_refreshed_list();
        // Pick the mounted volume whose mount point is the longest ancestor of dir.
        let mut best: Option<u64> = None;
        let mut best_len = 0usize;
        for disk in disks.list() {
            let mp = disk.mount_point();
            if dir.starts_with(mp) {
                let len = mp.as_os_str().len();
                if len >= best_len {
                    best_len = len;
                    best = Some(disk.available_space());
                }
            }
        }
        best.unwrap_or(u64::MAX)
    }
}

/// Attachment blob store rooted at the app data directory.
#[derive(Clone)]
pub struct DiskBlobStore {
    root: PathBuf,
    disk: Arc<dyn DiskSpaceProbe>,
}

impl DiskBlobStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            disk: Arc::new(SysinfoDiskSpace),
        }
    }

    /// Construct with a custom space probe (tests).
    pub fn with_probe(root: PathBuf, disk: Arc<dyn DiskSpaceProbe>) -> Self {
        Self { root, disk }
    }

    /// Refuse a write that would breach the low-watermark or lacks `needed` space.
    pub fn check_free_space(&self, dir: &Path, needed_bytes: u64) -> AppResult<()> {
        let avail = self.disk.available_bytes(dir);
        if avail < needed_bytes.max(LOW_WATERMARK_BYTES) {
            return Err(AppError::FsDiskFull);
        }
        Ok(())
    }

    fn relative_path(account: &str, year: u32, month: u32, mail: &str, filename: &str) -> String {
        // Defang the filename: keep the final path component only.
        let safe = Path::new(filename)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "attachment.bin".into());
        format!("{account}/attachments/{year:04}/{month:02}/{mail}/{safe}")
    }

    fn abs(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// Write a small attachment from memory; returns its relative path + sha.
    pub async fn write_attachment(
        &self,
        account: &str,
        mail: &str,
        year: u32,
        month: u32,
        filename: &str,
        data: &[u8],
    ) -> AppResult<WriteResult> {
        if is_blocked_executable(filename) {
            return Err(AppError::Forbidden("executable attachment".into()));
        }
        let rel = Self::relative_path(account, year, month, mail, filename);
        let abs = self.abs(&rel);
        let parent = abs.parent().expect("attachment path has parent");
        self.check_free_space(
            parent.parent().unwrap_or(&self.root),
            (data.len() as u64) * 3 / 2,
        )?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::FsPermission(format!("mkdir attachment dir: {e}")))?;
        let tmp = abs.with_extension("part");
        tokio::fs::write(&tmp, data)
            .await
            .map_err(|e| AppError::FsPermission(format!("write attachment: {e}")))?;
        tokio::fs::rename(&tmp, &abs)
            .await
            .map_err(|e| AppError::FsPermission(format!("commit attachment: {e}")))?;
        Ok(WriteResult {
            relative_path: rel,
            sha256: sha256_of(data),
            bytes_written: data.len() as u64,
        })
    }

    /// Stream a (potentially large) attachment in 64 KB chunks, computing the
    /// SHA-256 as it goes — never buffering the whole file (F_A5 §5.2).
    #[allow(clippy::too_many_arguments)]
    pub async fn write_attachment_stream<R>(
        &self,
        account: &str,
        mail: &str,
        year: u32,
        month: u32,
        filename: &str,
        expected_size: u64,
        mut reader: R,
    ) -> AppResult<WriteResult>
    where
        R: AsyncRead + Unpin,
    {
        if is_blocked_executable(filename) {
            return Err(AppError::Forbidden("executable attachment".into()));
        }
        let rel = Self::relative_path(account, year, month, mail, filename);
        let abs = self.abs(&rel);
        let parent = abs.parent().expect("attachment path has parent");
        self.check_free_space(parent.parent().unwrap_or(&self.root), expected_size * 3 / 2)?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::FsPermission(format!("mkdir attachment dir: {e}")))?;

        let tmp = abs.with_extension("part");
        let mut file = tokio::fs::File::create(&tmp)
            .await
            .map_err(|e| AppError::FsPermission(format!("create attachment: {e}")))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; CHUNK];
        let mut total: u64 = 0;
        loop {
            let n = reader
                .read(&mut buf)
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("read attachment stream: {e}")))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])
                .await
                .map_err(|e| AppError::FsPermission(format!("write attachment chunk: {e}")))?;
            total += n as u64;
        }
        file.flush()
            .await
            .map_err(|e| AppError::FsPermission(format!("flush attachment: {e}")))?;
        drop(file);
        tokio::fs::rename(&tmp, &abs)
            .await
            .map_err(|e| AppError::FsPermission(format!("commit attachment: {e}")))?;

        Ok(WriteResult {
            relative_path: rel,
            sha256: hex(&hasher.finalize()),
            bytes_written: total,
        })
    }

    pub async fn read_attachment(&self, relative_path: &str) -> AppResult<Vec<u8>> {
        tokio::fs::read(self.abs(relative_path))
            .await
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => AppError::NotFound,
                std::io::ErrorKind::PermissionDenied => AppError::FsPermission(e.to_string()),
                _ => AppError::Internal(anyhow::anyhow!("read attachment: {e}")),
            })
    }

    /// Delete a file. A missing file is success (orphan-tolerant, T026 §6).
    pub async fn delete_attachment(&self, relative_path: &str) -> AppResult<()> {
        match tokio::fs::remove_file(self.abs(relative_path)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(AppError::FsPermission(format!("delete attachment: {e}"))),
        }
    }

    /// Create a hard link from an existing blob to a new relative path (dedup,
    /// F_A5 §5.4). Returns the new relative path.
    pub async fn hard_link(
        &self,
        existing_relative: &str,
        account: &str,
        mail: &str,
        year: u32,
        month: u32,
        filename: &str,
    ) -> AppResult<String> {
        let new_rel = Self::relative_path(account, year, month, mail, filename);
        let new_abs = self.abs(&new_rel);
        if let Some(parent) = new_abs.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AppError::FsPermission(format!("mkdir for hardlink: {e}")))?;
        }
        // remove_file ignores missing; then hard_link.
        let _ = tokio::fs::remove_file(&new_abs).await;
        std::fs::hard_link(self.abs(existing_relative), &new_abs)
            .map_err(|e| AppError::FsPermission(format!("hard link: {e}")))?;
        Ok(new_rel)
    }

    /// Absolute path for a relative blob path (used by open/reveal, T026).
    pub fn absolute(&self, relative_path: &str) -> PathBuf {
        self.abs(relative_path)
    }

    /// Sum bytes for an account: filesystem attachments + DB-resident body HTML.
    pub async fn account_disk_usage(&self, account: &str, db: &Db) -> AppResult<DiskUsage> {
        let account_dir = self.root.join(account).join("attachments");
        let attachment_bytes = tokio::task::spawn_blocking(move || dir_size(&account_dir))
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("walkdir join: {e}")))?;

        let (body,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(length(body_html)), 0) FROM mails WHERE account_id = ?",
        )
        .bind(account)
        .fetch_one(db.pool())
        .await
        .map_err(super::map_sqlx_err)?;
        let body_bytes = body.max(0) as u64;

        Ok(DiskUsage {
            total_bytes: attachment_bytes + body_bytes,
            attachment_bytes,
            body_bytes,
        })
    }

    /// Remove an account's entire attachment tree (orphan cleanup, T026 §3).
    /// Returns bytes freed.
    pub async fn cleanup_account_dir(&self, account: &str) -> AppResult<u64> {
        let account_dir = self.root.join(account);
        let freed = {
            let d = account_dir.clone();
            tokio::task::spawn_blocking(move || dir_size(&d.join("attachments")))
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("walkdir join: {e}")))?
        };
        match tokio::fs::remove_dir_all(&account_dir).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(AppError::FsPermission(format!("remove account dir: {e}"))),
        }
        let _ = now_unix();
        Ok(freed)
    }
}

/// Recursive byte total of a directory tree (0 if missing).
fn dir_size(dir: &Path) -> u64 {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedSpace(u64);
    impl DiskSpaceProbe for FixedSpace {
        fn available_bytes(&self, _dir: &Path) -> u64 {
            self.0
        }
    }

    fn store(tmp: &Path, free: u64) -> DiskBlobStore {
        DiskBlobStore::with_probe(tmp.to_path_buf(), Arc::new(FixedSpace(free)))
    }

    #[tokio::test]
    async fn write_read_delete_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path(), u64::MAX);
        let res = s
            .write_attachment("acc", "mail", 2026, 6, "report.pdf", b"hello pdf")
            .await
            .unwrap();
        assert!(res.relative_path.ends_with("report.pdf"));
        assert_eq!(
            s.read_attachment(&res.relative_path).await.unwrap(),
            b"hello pdf"
        );
        s.delete_attachment(&res.relative_path).await.unwrap();
        assert!(matches!(
            s.read_attachment(&res.relative_path).await.unwrap_err(),
            AppError::NotFound
        ));
        // Deleting a missing file is still Ok (orphan tolerant).
        s.delete_attachment(&res.relative_path).await.unwrap();
    }

    #[tokio::test]
    async fn executable_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path(), u64::MAX);
        for name in ["virus.exe", "Setup.MSI", "run.sh"] {
            assert!(matches!(
                s.write_attachment("a", "m", 2026, 6, name, b"x")
                    .await
                    .unwrap_err(),
                AppError::Forbidden(_)
            ));
        }
        // A normal document is allowed.
        assert!(s
            .write_attachment("a", "m", 2026, 6, "ok.pdf", b"x")
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn low_watermark_blocks_write() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path(), 10 * 1024 * 1024); // 10 MB < 500 MB floor
        assert!(matches!(
            s.write_attachment("a", "m", 2026, 6, "ok.pdf", b"x")
                .await
                .unwrap_err(),
            AppError::FsDiskFull
        ));
    }

    #[tokio::test]
    async fn sha256_matches_known_vector() {
        // echo -n "abc" | sha256sum
        assert_eq!(
            sha256_of(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[tokio::test]
    async fn stream_write_computes_sha_and_size() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path(), u64::MAX);
        let data = vec![7u8; 200_000]; // > one chunk
        let res = s
            .write_attachment_stream("a", "m", 2026, 6, "big.bin", data.len() as u64, &data[..])
            .await
            .unwrap();
        assert_eq!(res.bytes_written, 200_000);
        assert_eq!(res.sha256, sha256_of(&data));
    }
}
