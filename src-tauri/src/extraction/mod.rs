//! Attachment text extraction pipeline (T108, A5 deepening · v0.6).
//!
//! Turns a downloaded attachment file on disk into plain text in
//! `attachments.extracted_text`, advancing `attachments.extraction_status`:
//!
//! * `pending`  — downloaded but not yet processed (migration 011 default).
//! * `indexed`  — text recovered and stored.
//! * `skipped`  — unsupported MIME (video / audio / archive / executable / image),
//!   legacy `.doc`, or an image-only (scanned) PDF with no text layer.
//! * `error`    — the file was the right type but parsing failed (corrupt /
//!   password-protected / panicked parser).
//!
//! Hard rules (T108 §6):
//! * every synchronous parser runs inside `spawn_blocking` (CPU-bound, never on
//!   the async executor) and inside `catch_unwind` (a panicking parser marks one
//!   row `error`, never crashes the batch);
//! * `content_type` decides the parser, with the filename extension as a fallback
//!   only when the MIME is generic (`application/octet-stream`);
//! * extracted text is capped at [`MAX_EXTRACTED_BYTES`] (200 KB) before storage;
//! * `extract_pending_batch` caps live parsers at [`EXTRACTION_CONCURRENCY`].
//!
//! What this card does NOT do: FTS5 / vector indexing (that is T109, which
//! consumes the `indexed` rows), OCR, or virus scanning.

pub mod index;
pub mod office;
pub mod pdf;
pub mod plaintext;

use std::sync::Arc;

use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::error::{AppError, AppResult};
use crate::storage::{facade::StorageFacade, map_sqlx_err};
use crate::util::now_unix;

/// Largest `extracted_text` written to SQLite — 200 KB UTF-8 (T108 §3b).
pub const MAX_EXTRACTED_BYTES: usize = 200 * 1024;

/// Concurrent parsers inside one [`ExtractionService::extract_pending_batch`]
/// (T108 §6). Caps simultaneous CPU + I/O so a backfill never saturates the box.
pub const EXTRACTION_CONCURRENCY: usize = 4;

/// Images larger than this are skipped outright (no OCR; F_A5 §9). Smaller images
/// are skipped too — the threshold only documents the "never read the bytes" rule.
pub const IMAGE_SKIP_SIZE: i64 = 10 * 1024 * 1024;

/// Outcome of one attachment extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtractionOutcome {
    /// Text recovered; `chars` is the stored length (after truncation).
    Indexed { chars: usize },
    /// Unsupported type or image-only PDF — nothing to index.
    Skipped { reason: &'static str },
    /// The file was a supported type but parsing failed.
    Error { message: String },
}

/// Aggregate result of a batch run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExtractionStats {
    pub indexed: u32,
    pub skipped: u32,
    pub error: u32,
    pub elapsed_ms: u64,
}

/// Which parser a blob is routed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Pdf,
    Docx,
    Pptx,
    Spreadsheet,
    Plain,
    /// Deliberately not extracted; the `&str` is the stable skip reason.
    Skip(&'static str),
}

/// Parsed payload before storage.
enum Parsed {
    Text(String),
    NoText,
}

/// One attachment's extraction-relevant columns.
#[derive(sqlx::FromRow)]
struct ExtractRow {
    content_type: String,
    filename: String,
    local_path: Option<String>,
    downloaded: i64,
    size_bytes: i64,
}

/// Attachment text-extraction service. Cheap to clone (storage handles are
/// `Arc`-backed), so batch tasks each take their own clone.
#[derive(Clone)]
pub struct ExtractionService {
    storage: StorageFacade,
}

impl ExtractionService {
    pub fn new(storage: StorageFacade) -> Self {
        Self { storage }
    }

    /// Build from shared app state.
    pub fn from_state(state: &crate::state::AppState) -> Self {
        Self::new(state.storage.clone())
    }

    /// Number of downloaded-but-unextracted attachments (the backfill queue size).
    pub async fn pending_count(&self) -> AppResult<u32> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM attachments WHERE extraction_status = 'pending' AND downloaded = 1",
        )
        .fetch_one(self.storage.db().pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(n.max(0) as u32)
    }

    /// Extract up to `limit` pending attachments, [`EXTRACTION_CONCURRENCY`] at a
    /// time. Per-row failures are recorded as `error` and counted, never bubbled.
    pub async fn extract_pending_batch(&self, limit: usize) -> AppResult<ExtractionStats> {
        let start = std::time::Instant::now();
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM attachments WHERE extraction_status = 'pending' AND downloaded = 1 \
             ORDER BY created_at LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(self.storage.db().pool())
        .await
        .map_err(map_sqlx_err)?;

        let sem = Arc::new(Semaphore::new(EXTRACTION_CONCURRENCY));
        let mut set: JoinSet<ExtractionOutcome> = JoinSet::new();
        for id in ids {
            let permit = sem.clone().acquire_owned().await.expect("semaphore open");
            let me = self.clone();
            set.spawn(async move {
                let _permit = permit;
                me.extract_one(&id)
                    .await
                    .unwrap_or_else(|e| ExtractionOutcome::Error {
                        message: e.to_string(),
                    })
            });
        }

        let mut stats = ExtractionStats::default();
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok(ExtractionOutcome::Indexed { .. }) => stats.indexed += 1,
                Ok(ExtractionOutcome::Skipped { .. }) => stats.skipped += 1,
                Ok(ExtractionOutcome::Error { .. }) => stats.error += 1,
                Err(e) => {
                    tracing::warn!(error = %e, "extraction task join failed");
                    stats.error += 1;
                }
            }
        }
        stats.elapsed_ms = start.elapsed().as_millis() as u64;
        Ok(stats)
    }

    /// Extract one attachment by id. Reads bytes off the async runtime, parses in
    /// a blocking task guarded by `catch_unwind`, then writes the status column.
    pub async fn extract_one(&self, attachment_id: &str) -> AppResult<ExtractionOutcome> {
        let db = self.storage.db().pool();
        let row: ExtractRow = sqlx::query_as(
            "SELECT content_type, filename, local_path, downloaded, size_bytes \
             FROM attachments WHERE id = ?",
        )
        .bind(attachment_id)
        .fetch_optional(db)
        .await
        .map_err(map_sqlx_err)?
        .ok_or(AppError::NotFound)?;

        // Not yet on disk: leave it `pending` for a later pass (the trigger fires
        // extract_one only after download, so this is a defensive guard).
        if row.downloaded == 0 || row.local_path.is_none() {
            return Ok(ExtractionOutcome::Skipped {
                reason: "not_downloaded",
            });
        }
        let local_path = row.local_path.unwrap();

        // Route by MIME (extension as fallback) before touching the disk.
        let route = route(&row.content_type, &row.filename, row.size_bytes);
        if let Route::Skip(reason) = route {
            self.mark_skipped(attachment_id, reason).await?;
            return Ok(ExtractionOutcome::Skipped { reason });
        }

        // Read the bytes off the async runtime.
        let bytes = match self.storage.blobs().read_attachment(&local_path).await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(attachment_id, error = %e, "extraction: blob read failed");
                self.mark_error(attachment_id).await?;
                return Ok(ExtractionOutcome::Error {
                    message: e.to_string(),
                });
            }
        };

        // Parse on a blocking thread, inside catch_unwind (parsers can panic).
        let parsed = tokio::task::spawn_blocking(move || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parse_blob(route, &bytes)))
        })
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("extraction task join: {e}")))?;

        match parsed {
            Ok(Ok(Parsed::Text(text))) => {
                let text = truncate_to_bytes(text, MAX_EXTRACTED_BYTES);
                let chars = text.len();
                self.mark_indexed(attachment_id, &text).await?;
                Ok(ExtractionOutcome::Indexed { chars })
            }
            Ok(Ok(Parsed::NoText)) => {
                // Parsed fine but no text layer (scanned PDF) — skip with sentinel.
                self.mark_skipped_with_text(attachment_id, pdf::NO_TEXT_SENTINEL)
                    .await?;
                Ok(ExtractionOutcome::Skipped {
                    reason: "no_text_layer",
                })
            }
            Ok(Err(e)) => {
                tracing::warn!(attachment_id, error = %e, "extraction: parse failed");
                self.mark_error(attachment_id).await?;
                Ok(ExtractionOutcome::Error {
                    message: e.to_string(),
                })
            }
            Err(_) => {
                tracing::warn!(attachment_id, "extraction: parser panicked");
                self.mark_error(attachment_id).await?;
                Ok(ExtractionOutcome::Error {
                    message: "parser panicked".into(),
                })
            }
        }
    }

    async fn mark_indexed(&self, id: &str, text: &str) -> AppResult<()> {
        sqlx::query(
            "UPDATE attachments SET extraction_status = 'indexed', extracted_text = ?, \
             extracted_at = ? WHERE id = ?",
        )
        .bind(text)
        .bind(now_unix())
        .bind(id)
        .execute(self.storage.db().pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    async fn mark_skipped(&self, id: &str, _reason: &str) -> AppResult<()> {
        sqlx::query(
            "UPDATE attachments SET extraction_status = 'skipped', extracted_at = ? WHERE id = ?",
        )
        .bind(now_unix())
        .bind(id)
        .execute(self.storage.db().pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    async fn mark_skipped_with_text(&self, id: &str, text: &str) -> AppResult<()> {
        sqlx::query(
            "UPDATE attachments SET extraction_status = 'skipped', extracted_text = ?, \
             extracted_at = ? WHERE id = ?",
        )
        .bind(text)
        .bind(now_unix())
        .bind(id)
        .execute(self.storage.db().pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    async fn mark_error(&self, id: &str) -> AppResult<()> {
        sqlx::query(
            "UPDATE attachments SET extraction_status = 'error', extracted_at = ? WHERE id = ?",
        )
        .bind(now_unix())
        .bind(id)
        .execute(self.storage.db().pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }
}

/// Decide which parser handles a blob. `content_type` wins; the filename
/// extension is consulted only when the MIME is generic or empty (T108 §6).
fn route(content_type: &str, filename: &str, size_bytes: i64) -> Route {
    let ct = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    // Never read these — fast skip without disk I/O (T108 §6).
    if ct.starts_with("video/") || ct.starts_with("audio/") {
        return Route::Skip("media");
    }
    if ct.starts_with("image/") {
        // Images are never OCR'd; the size threshold only documents the rule.
        let _ = size_bytes >= IMAGE_SKIP_SIZE;
        return Route::Skip("image");
    }
    if is_archive_mime(&ct) {
        return Route::Skip("archive");
    }
    if is_executable_mime(&ct) {
        return Route::Skip("executable");
    }

    // Definite content-type routes.
    match ct.as_str() {
        "application/pdf" => return Route::Pdf,
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            return Route::Docx
        }
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
            return Route::Pptx
        }
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        | "application/vnd.ms-excel"
        | "application/vnd.oasis.opendocument.spreadsheet" => return Route::Spreadsheet,
        "application/msword" => return Route::Skip("legacy_doc"),
        "application/vnd.ms-powerpoint" => return Route::Skip("legacy_ppt"),
        _ => {}
    }
    if ct.starts_with("text/")
        || matches!(
            ct.as_str(),
            "application/json"
                | "application/xml"
                | "application/yaml"
                | "application/x-yaml"
                | "application/xhtml+xml"
                | "application/csv"
        )
    {
        return Route::Plain;
    }

    // Generic / unknown MIME → fall back to the filename extension.
    route_by_extension(filename)
}

/// Extension fallback (lower-cased final suffix).
fn route_by_extension(filename: &str) -> Route {
    let ext = filename
        .rsplit('.')
        .next()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "pdf" => Route::Pdf,
        "docx" => Route::Docx,
        "pptx" => Route::Pptx,
        "xlsx" | "xls" | "xlsm" | "ods" => Route::Spreadsheet,
        "doc" => Route::Skip("legacy_doc"),
        "ppt" => Route::Skip("legacy_ppt"),
        "txt" | "md" | "markdown" | "csv" | "tsv" | "json" | "xml" | "yaml" | "yml" | "html"
        | "htm" | "log" | "text" => Route::Plain,
        _ => Route::Skip("unsupported"),
    }
}

fn is_archive_mime(ct: &str) -> bool {
    matches!(
        ct,
        "application/zip"
            | "application/x-zip-compressed"
            | "application/x-rar-compressed"
            | "application/vnd.rar"
            | "application/x-7z-compressed"
            | "application/gzip"
            | "application/x-gzip"
            | "application/x-tar"
            | "application/x-bzip2"
    )
}

fn is_executable_mime(ct: &str) -> bool {
    matches!(
        ct,
        "application/x-executable"
            | "application/x-msdownload"
            | "application/x-msdos-program"
            | "application/vnd.microsoft.portable-executable"
            | "application/x-sh"
            | "application/x-mach-binary"
    )
}

/// Run the routed parser. Pure (no I/O beyond the in-memory bytes).
fn parse_blob(route: Route, bytes: &[u8]) -> AppResult<Parsed> {
    match route {
        Route::Pdf => match pdf::extract_pdf(bytes)? {
            pdf::PdfOutcome::Text(t) => Ok(Parsed::Text(t)),
            pdf::PdfOutcome::NoText => Ok(Parsed::NoText),
        },
        Route::Docx => Ok(Parsed::Text(office::extract_docx(bytes)?)),
        Route::Pptx => Ok(Parsed::Text(office::extract_pptx(bytes)?)),
        Route::Spreadsheet => Ok(Parsed::Text(office::extract_spreadsheet(bytes)?)),
        Route::Plain => Ok(Parsed::Text(plaintext::extract_plaintext(bytes)?)),
        // Skip is handled before parsing; treat as a no-op skip if it reaches here.
        Route::Skip(_) => Ok(Parsed::NoText),
    }
}

/// Truncate a string to at most `max` bytes, snapping back to a UTF-8 boundary.
fn truncate_to_bytes(s: String, max: usize) -> String {
    if s.len() <= max {
        return s;
    }
    tracing::warn!(
        len = s.len(),
        cap = max,
        "extracted text exceeds cap; truncating"
    );
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_by_content_type() {
        assert_eq!(route("application/pdf", "x.bin", 10), Route::Pdf);
        assert_eq!(
            route(
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                "x",
                10
            ),
            Route::Spreadsheet
        );
        assert!(matches!(route("video/mp4", "clip.mp4", 10), Route::Skip(_)));
        assert!(matches!(
            route("application/zip", "a.zip", 10),
            Route::Skip(_)
        ));
        assert!(matches!(route("image/png", "a.png", 10), Route::Skip(_)));
        assert!(matches!(
            route("application/msword", "a.doc", 10),
            Route::Skip(_)
        ));
    }

    #[test]
    fn routes_generic_mime_by_extension() {
        assert_eq!(
            route("application/octet-stream", "notes.md", 10),
            Route::Plain
        );
        assert_eq!(
            route("application/octet-stream", "deck.pptx", 10),
            Route::Pptx
        );
        assert!(matches!(
            route("application/octet-stream", "movie.mkv", 10),
            Route::Skip(_)
        ));
    }

    #[test]
    fn truncation_respects_cap_and_char_boundary() {
        let s = "é".repeat(200 * 1024); // 2 bytes each → 400 KB
        let out = truncate_to_bytes(s, MAX_EXTRACTED_BYTES);
        assert!(out.len() <= MAX_EXTRACTED_BYTES);
        // Must still be valid UTF-8 (no broken multi-byte char at the cut).
        assert!(std::str::from_utf8(out.as_bytes()).is_ok());
    }

    #[tokio::test]
    async fn extract_one_indexes_a_plaintext_attachment() {
        use crate::storage::AttachmentRepo;
        let storage = StorageFacade::open_in_memory().await.unwrap();
        seed_account_mail(&storage, "acc", "m1").await;

        // Insert a pending attachment row and write its bytes to the blob store.
        let written = storage
            .blobs()
            .write_attachment(
                "acc",
                "m1",
                2026,
                6,
                "notes.txt",
                b"quarterly budget summary",
            )
            .await
            .unwrap();
        let att_id = crate::util::new_uuid();
        sqlx::query(
            "INSERT INTO attachments (id, mail_id, account_id, filename, content_type, size_bytes, \
             downloaded, local_path, is_inline, extraction_status, created_at) \
             VALUES (?, 'm1', 'acc', 'notes.txt', 'text/plain', 24, 1, ?, 0, 'pending', 0)",
        )
        .bind(&att_id)
        .bind(&written.relative_path)
        .execute(storage.db().pool())
        .await
        .unwrap();

        let svc = ExtractionService::new(storage.clone());
        let outcome = svc.extract_one(&att_id).await.unwrap();
        assert!(matches!(outcome, ExtractionOutcome::Indexed { .. }));

        let (status, text): (String, Option<String>) = sqlx::query_as(
            "SELECT extraction_status, extracted_text FROM attachments WHERE id = ?",
        )
        .bind(&att_id)
        .fetch_one(storage.db().pool())
        .await
        .unwrap();
        assert_eq!(status, "indexed");
        assert!(text.unwrap().contains("budget"));
        // AttachmentRepo still reads the row fine after the new columns.
        assert!(AttachmentRepo::new(storage.db()).get(&att_id).await.is_ok());
    }

    #[tokio::test]
    async fn extract_pending_batch_counts_outcomes() {
        let storage = StorageFacade::open_in_memory().await.unwrap();
        seed_account_mail(&storage, "acc", "m1").await;

        // One indexable text file + one skipped video (no bytes needed).
        let w = storage
            .blobs()
            .write_attachment("acc", "m1", 2026, 6, "a.txt", b"hello world contract")
            .await
            .unwrap();
        insert_att(&storage, "att-txt", &w.relative_path, "text/plain", true).await;
        insert_att(&storage, "att-vid", "acc/x/clip.mp4", "video/mp4", true).await;

        let svc = ExtractionService::new(storage.clone());
        let stats = svc.extract_pending_batch(10).await.unwrap();
        assert_eq!(stats.indexed, 1);
        assert_eq!(stats.skipped, 1);
        assert_eq!(stats.error, 0);
        // Queue is now drained.
        assert_eq!(svc.pending_count().await.unwrap(), 0);
    }

    async fn seed_account_mail(storage: &StorageFacade, acc: &str, mail: &str) {
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, created_at, updated_at) \
             VALUES (?, 'a@x.com', 'A', 'imap', 'slate', 'W', 0, 0)",
        )
        .bind(acc)
        .execute(storage.db().pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, from_email, to_addrs, date_sent, date_received, subject, created_at, updated_at) \
             VALUES (?, ?, ?, 'a@x.com', '[]', 1000, 1000, 'S', 0, 0)",
        )
        .bind(mail)
        .bind(acc)
        .bind(format!("<{mail}@x>"))
        .execute(storage.db().pool())
        .await
        .unwrap();
    }

    async fn insert_att(
        storage: &StorageFacade,
        id: &str,
        local_path: &str,
        content_type: &str,
        downloaded: bool,
    ) {
        sqlx::query(
            "INSERT INTO attachments (id, mail_id, account_id, filename, content_type, size_bytes, \
             downloaded, local_path, is_inline, extraction_status, created_at) \
             VALUES (?, 'm1', 'acc', ?, ?, 100, ?, ?, 0, 'pending', 0)",
        )
        .bind(id)
        .bind(format!("{id}.bin"))
        .bind(content_type)
        .bind(downloaded as i64)
        .bind(local_path)
        .execute(storage.db().pool())
        .await
        .unwrap();
    }
}
