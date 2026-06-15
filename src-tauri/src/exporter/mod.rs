//! Export pipeline (T052) — per-account mail export to mbox / JSON Lines,
//! packaged as a ZIP bundle with a `MANIFEST.json`.
//!
//! `spawn_export` validates, estimates disk use (refusing with `FS_DISK_FULL`
//! when target free space < estimate × 1.1, F_H2 §6), registers a cancellable
//! handle, and runs the streaming task on Tokio. Mails are read in batches of
//! [`EXPORT_BATCH_SIZE`] so a 100k-mail export never loads the corpus into
//! memory (F_H2 §4.1). Progress is emitted per batch as `export:progress`.
//!
//! Log safety (09 §5): every log line in this module carries identifiers and
//! counts only — never subjects, bodies, or addresses.

pub mod json_lines;
pub mod mbox;
pub mod zip;

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;
use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::types::{ExportFormat, StartExportParams};
use crate::util::{new_uuid, now_unix};

/// Batch size for the paged `mails` reads (T052 §6).
pub const EXPORT_BATCH_SIZE: i64 = 500;
/// Free-space safety factor over the size estimate (F_H2 §6).
const DISK_SAFETY_FACTOR: f64 = 1.1;

// ── Task registry ─────────────────────────────────────────────────────────────

/// One live (or finished) export task.
#[derive(Clone)]
pub struct ExportHandle {
    pub cancel: Arc<AtomicBool>,
    pub output_dir: PathBuf,
}

static REGISTRY: Lazy<Mutex<HashMap<String, ExportHandle>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn register(task_id: &str, handle: ExportHandle) {
    REGISTRY
        .lock()
        .expect("export registry poisoned")
        .insert(task_id.to_string(), handle);
}

/// Look up a task (used by cancel + the open-output command).
pub fn handle_for(task_id: &str) -> Option<ExportHandle> {
    REGISTRY
        .lock()
        .expect("export registry poisoned")
        .get(task_id)
        .cloned()
}

/// Flag a task for cancellation. Already-written files stay on disk (F_H2 §4.1).
pub fn request_cancel(task_id: &str) -> AppResult<()> {
    match handle_for(task_id) {
        Some(h) => {
            h.cancel.store(true, Ordering::SeqCst);
            tracing::info!(
                event = "export_cancel_requested",
                task_id = task_id,
                "export cancel"
            );
            Ok(())
        }
        None => Err(AppError::NotFound),
    }
}

// ── Shared row / recipient helpers ────────────────────────────────────────────

/// The column set every export format consumes. Credential-free by construction.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ExportRow {
    pub id: String,
    pub message_id: String,
    pub in_reply_to: Option<String>,
    pub subject: String,
    pub from_name: Option<String>,
    pub from_email: String,
    pub to_addrs: String,
    pub cc_addrs: String,
    pub date_sent: i64,
    pub body_text: Option<String>,
    pub has_attachments: i64,
}

/// Minimal recipient shape used in JSONL output and header reconstruction.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Recipient {
    pub name: Option<String>,
    pub email: String,
}

/// Parse the `to_addrs`/`cc_addrs` JSON column; malformed rows yield `[]`.
pub fn parse_recipients(json: &str) -> Vec<Recipient> {
    serde_json::from_str::<Vec<Recipient>>(json).unwrap_or_default()
}

/// `Name <email>, …` header rendering.
pub fn recipients_header(json: &str) -> String {
    parse_recipients(json)
        .into_iter()
        .map(|r| match r.name {
            Some(n) if !n.is_empty() => format!("{} <{}>", n, r.email),
            _ => r.email,
        })
        .collect::<Vec<_>>()
        .join(", ")
}

// ── Disk estimate ─────────────────────────────────────────────────────────────

async fn estimate_bytes(state: &AppState, params: &StartExportParams) -> AppResult<u64> {
    let pool = state.storage.db().pool();
    let mut total: i64 = 0;
    for account_id in &params.account_ids {
        let (body,): (Option<i64>,) = sqlx::query_as(
            "SELECT SUM(LENGTH(COALESCE(body_text,'')) + LENGTH(COALESCE(subject,'')) + 512) \
             FROM mails WHERE account_id = ? AND is_deleted = 0 \
             AND date_sent >= COALESCE(?, -9223372036854775808) \
             AND date_sent <= COALESCE(?, 9223372036854775807)",
        )
        .bind(account_id)
        .bind(params.date_from)
        .bind(params.date_to)
        .fetch_one(pool)
        .await
        .map_err(crate::storage::map_sqlx_err)?;
        total += body.unwrap_or(0);

        if params.include_attachments {
            let (att,): (Option<i64>,) = sqlx::query_as(
                "SELECT SUM(a.size_bytes) FROM attachments a \
                 JOIN mails m ON m.id = a.mail_id \
                 WHERE m.account_id = ? AND m.is_deleted = 0 AND a.downloaded = 1",
            )
            .bind(account_id)
            .fetch_one(pool)
            .await
            .map_err(crate::storage::map_sqlx_err)?;
            total += att.unwrap_or(0);
        }
    }
    // The zip is STORED, so the bundle ≈ inputs; double for staging + bundle.
    Ok((total.max(0) as u64) * 2)
}

/// Best-effort free-space lookup for the disk holding `path`.
fn available_space(path: &Path) -> Option<u64> {
    use sysinfo::Disks;
    let disks = Disks::new_with_refreshed_list();
    let mut best: Option<(usize, u64)> = None;
    for disk in disks.list() {
        let mount = disk.mount_point();
        if path.starts_with(mount) {
            let depth = mount.components().count();
            if best.map(|(d, _)| depth > d).unwrap_or(true) {
                best = Some((depth, disk.available_space()));
            }
        }
    }
    best.map(|(_, avail)| avail)
}

// ── Spawn + run ───────────────────────────────────────────────────────────────

/// Validate, disk-check, register, and launch the export. Returns the task id
/// immediately (02: long tasks return a handle and stream events).
pub async fn spawn_export(state: AppState, params: StartExportParams) -> AppResult<String> {
    if params.account_ids.is_empty() {
        return Err(AppError::Validation("select at least one account".into()));
    }

    let output_dir = state
        .paths
        .root
        .join("exports")
        .join(format!("export-{}", now_unix()));
    std::fs::create_dir_all(&output_dir)
        .map_err(|e| AppError::FsPermission(format!("create export dir: {e}")))?;

    // Disk-space guard (F_H2 §6).
    let estimate = estimate_bytes(&state, &params).await?;
    if let Some(avail) = available_space(&output_dir) {
        if (avail as f64) < (estimate as f64) * DISK_SAFETY_FACTOR {
            return Err(AppError::FsDiskFull);
        }
    }

    let task_id = new_uuid();
    let cancel = Arc::new(AtomicBool::new(false));
    register(
        &task_id,
        ExportHandle {
            cancel: cancel.clone(),
            output_dir: output_dir.clone(),
        },
    );

    tracing::info!(
        event = "export_started",
        task_id = %task_id,
        account_count = params.account_ids.len(),
        format = params.format.as_wire(),
        "export task starting"
    );

    let tid = task_id.clone();
    tauri::async_runtime::spawn(async move {
        match run_export(&state, &tid, &params, &output_dir, &cancel).await {
            Ok(Some((zip_path, mail_count))) => {
                tracing::info!(
                    event = "export_complete",
                    task_id = %tid,
                    count = mail_count,
                    "export finished"
                );
                state.events.export_complete(
                    &tid,
                    &zip_path.display().to_string(),
                    &output_dir.display().to_string(),
                    mail_count,
                );
            }
            Ok(None) => {
                // Cancelled — partial files intentionally left in place.
                tracing::info!(event = "export_cancelled", task_id = %tid, "export cancelled");
            }
            Err(err) => {
                let code = err.code();
                state.events.export_error(&tid, code, "export failed");
            }
        }
    });

    Ok(task_id)
}

/// `Ok(Some(_))` = success, `Ok(None)` = cancelled.
async fn run_export(
    state: &AppState,
    task_id: &str,
    params: &StartExportParams,
    output_dir: &Path,
    cancel: &AtomicBool,
) -> AppResult<Option<(PathBuf, u64)>> {
    let pool = state.storage.db().pool();

    // Total for progress (one COUNT up front, T052 §6).
    let mut total: u64 = 0;
    for account_id in &params.account_ids {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM mails WHERE account_id = ? AND is_deleted = 0 \
             AND date_sent >= COALESCE(?, -9223372036854775808) \
             AND date_sent <= COALESCE(?, 9223372036854775807)",
        )
        .bind(account_id)
        .bind(params.date_from)
        .bind(params.date_to)
        .fetch_one(pool)
        .await
        .map_err(crate::storage::map_sqlx_err)?;
        total += n.max(0) as u64;
    }

    let mut files: Vec<String> = Vec::new();
    let mut processed: u64 = 0;
    let mut skipped_attachments: u64 = 0;
    let mut attachment_files: Vec<(String, PathBuf)> = Vec::new();

    for account_id in &params.account_ids {
        if cancel.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let ext = match params.format {
            ExportFormat::Mbox => "mbox",
            ExportFormat::Json => "jsonl",
        };
        let file_name = format!("account-{account_id}.{ext}");
        let file_path = output_dir.join(&file_name);
        let file = File::create(&file_path)
            .map_err(|e| AppError::FsPermission(format!("create export file: {e}")))?;
        let mut out = BufWriter::new(file);

        let mut offset: i64 = 0;
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Ok(None);
            }
            let rows: Vec<ExportRow> = sqlx::query_as(
                "SELECT id, message_id, in_reply_to, subject, from_name, from_email, \
                        to_addrs, cc_addrs, date_sent, body_text, has_attachments \
                 FROM mails WHERE account_id = ? AND is_deleted = 0 \
                 AND date_sent >= COALESCE(?, -9223372036854775808) \
                 AND date_sent <= COALESCE(?, 9223372036854775807) \
                 ORDER BY date_sent, id LIMIT ? OFFSET ?",
            )
            .bind(account_id)
            .bind(params.date_from)
            .bind(params.date_to)
            .bind(EXPORT_BATCH_SIZE)
            .bind(offset)
            .fetch_all(pool)
            .await
            .map_err(crate::storage::map_sqlx_err)?;

            if rows.is_empty() {
                break;
            }
            for row in &rows {
                match params.format {
                    ExportFormat::Mbox => mbox::write_mail(&mut out, row)?,
                    ExportFormat::Json => {
                        json_lines::write_mail(&mut out, row, params.include_body)?
                    }
                }
            }
            processed += rows.len() as u64;
            offset += rows.len() as i64;
            state
                .events
                .export_progress(task_id, processed, total, "mails");
        }
        out.flush()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("flush export file: {e}")))?;
        files.push(file_name);

        // Downloaded attachments only; missing ones are counted in the manifest
        // (MVP: no `--with-blobs` re-download, T052 §4).
        if params.include_attachments {
            let atts: Vec<(String, String, Option<String>, i64)> = sqlx::query_as(
                "SELECT a.mail_id, a.filename, a.local_path, a.downloaded \
                 FROM attachments a JOIN mails m ON m.id = a.mail_id \
                 WHERE m.account_id = ? AND m.is_deleted = 0",
            )
            .bind(account_id)
            .fetch_all(pool)
            .await
            .map_err(crate::storage::map_sqlx_err)?;

            for (mail_id, filename, local_path, downloaded) in atts {
                if cancel.load(Ordering::SeqCst) {
                    return Ok(None);
                }
                match (downloaded, local_path) {
                    (1, Some(rel)) => {
                        let abs = state.paths.root.join(&rel);
                        if abs.is_file() {
                            attachment_files
                                .push((format!("attachments/{mail_id}/{filename}"), abs));
                        } else {
                            skipped_attachments += 1;
                        }
                    }
                    _ => skipped_attachments += 1,
                }
            }
        }
    }

    // MANIFEST.json (JSON bundles always get one; mbox bundles too — it is the
    // machine-readable receipt either way).
    let manifest = json_lines::Manifest {
        format_version: json_lines::MANIFEST_FORMAT_VERSION,
        format: params.format.as_wire().to_string(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        account_ids: params.account_ids.clone(),
        mail_count: processed,
        include_body: params.include_body,
        include_attachments: params.include_attachments,
        skipped_attachments,
        files: files.clone(),
    };
    let manifest_path = output_dir.join("MANIFEST.json");
    {
        let file = File::create(&manifest_path)
            .map_err(|e| AppError::FsPermission(format!("create manifest: {e}")))?;
        let mut out = BufWriter::new(file);
        json_lines::write_manifest(&mut out, &manifest)?;
        out.flush()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("flush manifest: {e}")))?;
    }

    // ZIP packaging.
    state
        .events
        .export_progress(task_id, processed, total, "zip");
    let date_tag = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let zip_path = output_dir.join(format!("seekermail-export-{date_tag}.zip"));
    let mut zipw = zip::ZipWriter::create(&zip_path)?;
    zipw.add_file("MANIFEST.json", &manifest_path)?;
    for name in &files {
        zipw.add_file(name, &output_dir.join(name))?;
    }
    for (entry_name, abs) in &attachment_files {
        if cancel.load(Ordering::SeqCst) {
            return Ok(None);
        }
        zipw.add_file(entry_name, abs)?;
    }
    zipw.finish()?;

    Ok(Some((zip_path, processed)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipients_header_renders_names_and_plain_emails() {
        let json = r#"[{"name":"Ann","email":"ann@x.y"},{"name":null,"email":"bob@x.y"}]"#;
        assert_eq!(recipients_header(json), "Ann <ann@x.y>, bob@x.y");
        assert_eq!(recipients_header("not json"), "");
    }

    #[test]
    fn cancel_on_unknown_task_is_not_found() {
        let err = request_cancel("nope").unwrap_err();
        assert_eq!(err.code(), crate::types::ErrorCode::NotFound);
    }

    #[test]
    fn registry_roundtrip_and_cancel_flag() {
        let cancel = Arc::new(AtomicBool::new(false));
        register(
            "t-1",
            ExportHandle {
                cancel: cancel.clone(),
                output_dir: PathBuf::from("/tmp/x"),
            },
        );
        request_cancel("t-1").unwrap();
        assert!(cancel.load(Ordering::SeqCst));
        assert!(handle_for("t-1").is_some());
    }

    async fn seed_account_with_mails(state: &AppState, account_id: &str, n: usize) {
        let pool = state.storage.db().pool();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, created_at, updated_at) \
             VALUES (?, 'fixture@example.com', 'Fixture', 'imap', 'slate', 'W', 0, 0)",
        )
        .bind(account_id)
        .execute(pool)
        .await
        .unwrap();
        for i in 0..n {
            sqlx::query(
                "INSERT INTO mails (id, account_id, message_id, subject, from_email, to_addrs, \
                 date_sent, date_received, body_text, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, 'sender@example.com', \
                 '[{\"name\":null,\"email\":\"fixture@example.com\"}]', ?, ?, ?, 0, 0)",
            )
            .bind(format!("mail-{i}"))
            .bind(account_id)
            .bind(format!("<m{i}@example.com>"))
            .bind(format!("Fixture subject {i}"))
            .bind(1_700_000_000_i64 + i as i64)
            .bind(1_700_000_000_i64 + i as i64)
            .bind(format!("Body {i}\nFrom the fixture corpus"))
            .execute(pool)
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn exports_ten_fixture_mails_to_mbox_with_manifest_and_zip() {
        let (state, _rx) = AppState::test_state().await;
        seed_account_with_mails(&state, "acc-exp", 10).await;
        let dir = tempfile::tempdir().unwrap();

        let params = StartExportParams {
            account_ids: vec!["acc-exp".into()],
            date_from: None,
            date_to: None,
            format: ExportFormat::Mbox,
            include_body: true,
            include_attachments: false,
        };
        let cancel = AtomicBool::new(false);
        let (zip_path, count) = run_export(&state, "t-mbox", &params, dir.path(), &cancel)
            .await
            .unwrap()
            .expect("not cancelled");
        assert_eq!(count, 10);

        // mbox separator count == mail count (release-gate check, 05 §2.1 H2).
        let mbox = std::fs::read_to_string(dir.path().join("account-acc-exp.mbox")).unwrap();
        let separators = mbox
            .lines()
            .filter(|l| l.starts_with("From ") && !l.starts_with("From:"))
            .count();
        assert_eq!(separators, 10);
        // Body "From the fixture corpus" lines must be escaped, not separators.
        assert!(mbox.contains(">From the fixture corpus"));

        // Manifest is valid JSON with the right count.
        let manifest_raw = std::fs::read_to_string(dir.path().join("MANIFEST.json")).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&manifest_raw).unwrap();
        assert_eq!(manifest["mail_count"], 10);
        assert_eq!(manifest["format"], "mbox");

        // ZIP exists, is non-empty, structurally sound.
        assert!(zip_path.is_file());
        assert!(std::fs::metadata(&zip_path).unwrap().len() > 0);
        assert!(zip::looks_like_zip(&zip_path).unwrap());
    }

    #[tokio::test]
    async fn exports_jsonl_with_parseable_lines() {
        let (state, _rx) = AppState::test_state().await;
        seed_account_with_mails(&state, "acc-json", 3).await;
        let dir = tempfile::tempdir().unwrap();

        let params = StartExportParams {
            account_ids: vec!["acc-json".into()],
            date_from: None,
            date_to: None,
            format: ExportFormat::Json,
            include_body: false,
            include_attachments: false,
        };
        let cancel = AtomicBool::new(false);
        let (_zip, count) = run_export(&state, "t-json", &params, dir.path(), &cancel)
            .await
            .unwrap()
            .expect("not cancelled");
        assert_eq!(count, 3);

        let jsonl = std::fs::read_to_string(dir.path().join("account-acc-json.jsonl")).unwrap();
        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines.len(), 3);
        for line in lines {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            // include_body=false → body_text omitted (credentials are never present).
            assert!(v.get("body_text").is_none());
            assert!(v["id"].as_str().unwrap().starts_with("mail-"));
        }
    }

    #[tokio::test]
    async fn cancelled_export_stops_and_returns_none() {
        let (state, _rx) = AppState::test_state().await;
        seed_account_with_mails(&state, "acc-cancel", 5).await;
        let dir = tempfile::tempdir().unwrap();
        let params = StartExportParams {
            account_ids: vec!["acc-cancel".into()],
            date_from: None,
            date_to: None,
            format: ExportFormat::Mbox,
            include_body: true,
            include_attachments: false,
        };
        let cancel = AtomicBool::new(true); // pre-cancelled
        let out = run_export(&state, "t-c", &params, dir.path(), &cancel)
            .await
            .unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn spawn_export_rejects_empty_account_list() {
        let (state, _rx) = AppState::test_state().await;
        let err = spawn_export(
            state,
            StartExportParams {
                account_ids: vec![],
                date_from: None,
                date_to: None,
                format: ExportFormat::Mbox,
                include_body: true,
                include_attachments: false,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), crate::types::ErrorCode::Validation);
    }
}
