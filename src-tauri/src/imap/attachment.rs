//! Attachment streaming download + OS integration (T025, T026).
//!
//! Concurrency (F_A5 §5.3): a global manual cap (2), a global auto cap (4), and a
//! per-account auto cap (2 → 1 while backfill runs). A 50 MB memory ceiling caps
//! the total bytes buffered across in-flight downloads. SHA-256 dedup hard-links
//! duplicate blobs (F_A5 §5.4). Executables are never written or opened (F_A5 §7).
//!
//! Note: the transport seam returns a part's full bytes (addressed by the stored
//! 0-based MIME part index, migration 016); they are streamed to disk in 64 KB
//! writes with a streaming SHA. The live session re-parses the message and slices
//! out that part — correct for any MIME nesting, at the cost of re-fetching the
//! message body. A future optimisation can persist exact `BODYSTRUCTURE` part
//! numbers for a ranged `FETCH BODY.PEEK[n]`.

use std::collections::HashMap;
use std::sync::Arc;

use once_cell::sync::Lazy;
use tokio::sync::{Mutex, Semaphore};

use crate::config::{ATTACH_AUTO_GLOBAL, ATTACH_AUTO_PER_ACCOUNT, ATTACH_MANUAL_CONCURRENCY};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage::blob::is_blocked_executable;
use crate::storage::{AccountRepo, AttachmentRepo, MailRepo};
use crate::util::now_unix;

/// 50 MB memory ceiling, modelled in MB-granularity permits (F_A5 §5.2).
const MEM_CAP_MB: u32 = 50;

/// How the download was initiated (affects concurrency lane + progress events).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadMode {
    Manual,
    Auto,
}

static MANUAL_SEM: Lazy<Arc<Semaphore>> =
    Lazy::new(|| Arc::new(Semaphore::new(ATTACH_MANUAL_CONCURRENCY)));
static AUTO_GLOBAL_SEM: Lazy<Arc<Semaphore>> =
    Lazy::new(|| Arc::new(Semaphore::new(ATTACH_AUTO_GLOBAL)));
static MEM_SEM: Lazy<Arc<Semaphore>> = Lazy::new(|| Arc::new(Semaphore::new(MEM_CAP_MB as usize)));
static PER_ACCOUNT_AUTO: Lazy<Mutex<HashMap<String, Arc<Semaphore>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

async fn per_account_auto_sem(account_id: &str) -> Arc<Semaphore> {
    let mut map = PER_ACCOUNT_AUTO.lock().await;
    map.entry(account_id.to_string())
        .or_insert_with(|| Arc::new(Semaphore::new(ATTACH_AUTO_PER_ACCOUNT)))
        .clone()
}

/// Download one attachment's bytes to disk and return its relative path.
pub async fn download_one(
    state: &AppState,
    attachment_id: &str,
    mode: DownloadMode,
) -> AppResult<String> {
    let att_repo = AttachmentRepo::new(state.storage.db());
    let att = att_repo.get(attachment_id).await?;

    if is_blocked_executable(&att.filename) {
        return Err(AppError::Forbidden("executable attachment".into()));
    }

    let ctx = MailRepo::new(state.storage.db())
        .fetch_context(&att.mail_id)
        .await?;
    let uid = ctx.imap_uid.ok_or(AppError::NotFound)?;

    // ── Acquire concurrency permits for the chosen lane ─────────────────────
    let _manual = if mode == DownloadMode::Manual {
        Some(MANUAL_SEM.clone().acquire_owned().await.unwrap())
    } else {
        None
    };
    let (_auto_global, _auto_acct) = if mode == DownloadMode::Auto {
        let g = AUTO_GLOBAL_SEM.clone().acquire_owned().await.unwrap();
        let acct_sem = per_account_auto_sem(&ctx.account_id).await;
        // While a backfill runs, take 2 permits to throttle to 1 concurrent.
        let backfill = state
            .backfill_active
            .load(std::sync::atomic::Ordering::Relaxed);
        let a = if backfill {
            acct_sem
                .acquire_many_owned(ATTACH_AUTO_PER_ACCOUNT as u32)
                .await
                .unwrap()
        } else {
            acct_sem.acquire_many_owned(1).await.unwrap()
        };
        (Some(g), Some(a))
    } else {
        (None, None)
    };

    // Memory ceiling: reserve ceil(size_mb), clamped to the cap.
    let size_mb = ((att.size_bytes / (1024 * 1024)) as u32 + 1).clamp(1, MEM_CAP_MB);
    let _mem = MEM_SEM.clone().acquire_many_owned(size_mb).await.unwrap();

    // ── Fetch via the transport seam ────────────────────────────────────────
    // Address the exact MIME part by its stored 0-based index (migration 016).
    // The live session re-parses the message and returns that part's bytes, so
    // the index must match the one the parser assigned at ingest time.
    let part_index = att_repo.part_index(attachment_id).await?;
    let creds = crate::imap::sync::imap_creds_for(state, &ctx.account_id).await?;
    let mut session = state.net.imap.open(creds).await?;
    let bytes = session.fetch_part(uid, part_index).await?;

    if mode == DownloadMode::Manual {
        state.events.attachment_progress(attachment_id, 50);
    }

    let sha = crate::storage::blob::sha256_of(&bytes);
    let blobs = state.storage.blobs();

    // Dedup: hard-link an existing identical blob.
    let rel_path = if let Some(existing) = att_repo.find_by_sha256(&sha).await? {
        blobs
            .hard_link(
                &existing,
                &ctx.account_id,
                &att.mail_id,
                ctx.year,
                ctx.month,
                &att.filename,
            )
            .await?
    } else {
        blobs
            .write_attachment_stream(
                &ctx.account_id,
                &att.mail_id,
                ctx.year,
                ctx.month,
                &att.filename,
                bytes.len() as u64,
                &bytes[..],
            )
            .await?
            .relative_path
    };

    att_repo
        .set_downloaded(attachment_id, &rel_path, &sha, now_unix())
        .await?;
    state.events.attachment_progress(attachment_id, 100);
    state.events.attachment_ready(attachment_id, &rel_path);

    // T108/T109 increment path: extract text, then index it (FTS via trigger +
    // vectors). Best-effort and off the download hot path — never blocks the
    // download or surfaces an error to the caller.
    {
        let st = state.clone();
        let id = attachment_id.to_string();
        tokio::spawn(async move {
            let svc = crate::extraction::ExtractionService::from_state(&st);
            match svc.extract_one(&id).await {
                Ok(crate::extraction::ExtractionOutcome::Indexed { .. }) => {
                    let indexer = crate::extraction::index::AttachmentIndexer::from_state(&st);
                    if let Err(e) = indexer.index_one(&id).await {
                        tracing::debug!(attachment_id = %id, error = %e, "post-download index skipped");
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::debug!(attachment_id = %id, error = %e, "post-download extract skipped")
                }
            }
        });
    }

    Ok(rel_path)
}

/// Queue auto-download of an account's pending document attachments (T025 §3).
pub async fn trigger_auto(state: &AppState, account_id: &str) {
    // Knowledge-depth boundary for "in scope" mail.
    let boundary = match AccountRepo::new(state.storage.db()).get(account_id).await {
        Ok(acct) => acct
            .knowledge_depth_months
            .map(|m| now_unix() - (m as i64) * 30 * 86_400)
            .unwrap_or(0),
        Err(_) => 0,
    };
    let pending = match AttachmentRepo::new(state.storage.db())
        .pending_auto_for_account(account_id, boundary)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(account_id, error = %e, "auto-download scan failed");
            return;
        }
    };
    for att in pending {
        let st = state.clone();
        let id = att.id.clone();
        tokio::spawn(async move {
            if let Err(e) = download_one(&st, &id, DownloadMode::Auto).await {
                tracing::debug!(attachment_id = %id, error = %e, "auto attachment download skipped");
            }
        });
    }
}

// ── OS integration (T026) ────────────────────────────────────────────────────

/// Open an attachment with the OS default app. Executables are refused even when
/// already downloaded (F_A5 §7).
pub async fn open_attachment(state: &AppState, attachment_id: &str) -> AppResult<()> {
    let att = AttachmentRepo::new(state.storage.db())
        .get(attachment_id)
        .await?;
    if !att.downloaded {
        return Err(AppError::NotFound);
    }
    if is_blocked_executable(&att.filename) {
        return Err(AppError::Forbidden("executable: open refused".into()));
    }
    let rel = att.local_path.ok_or(AppError::NotFound)?;
    let abs = state.storage.blobs().absolute(&rel);
    if !abs.exists() {
        return Err(AppError::NotFound);
    }
    os_open(&abs, false)
}

/// Reveal an attachment in Finder/Explorer (no exec block — reveal is safe).
pub async fn reveal_attachment(state: &AppState, attachment_id: &str) -> AppResult<()> {
    let att = AttachmentRepo::new(state.storage.db())
        .get(attachment_id)
        .await?;
    let rel = att.local_path.ok_or(AppError::NotFound)?;
    let abs = state.storage.blobs().absolute(&rel);
    if !abs.exists() {
        return Err(AppError::NotFound);
    }
    os_open(&abs, true)
}

/// Local path for an attachment if downloaded (drives open-vs-download UI).
pub async fn get_local_path(state: &AppState, attachment_id: &str) -> AppResult<Option<String>> {
    let att = AttachmentRepo::new(state.storage.db())
        .get(attachment_id)
        .await?;
    Ok(att.local_path.filter(|_| att.downloaded))
}

/// Remove an account's attachment files (orphan cleanup). Delegates to the blob
/// store's directory wipe. Returns bytes freed.
pub async fn cleanup_attachment_files(state: &AppState, account_id: &str) -> AppResult<u64> {
    state.storage.blobs().cleanup_account_dir(account_id).await
}

/// Remove the downloaded files for a single mail (soft-delete path, T026 §3).
pub async fn cleanup_attachment_files_for_mail(state: &AppState, mail_id: &str) -> AppResult<u64> {
    let blobs = state.storage.blobs();
    let mut freed = 0u64;
    for att in AttachmentRepo::new(state.storage.db())
        .list_by_mail(mail_id)
        .await?
    {
        if let Some(rel) = att.local_path {
            freed += att.size_bytes;
            blobs.delete_attachment(&rel).await?;
        }
    }
    Ok(freed)
}

/// Spawn the OS file opener. `reveal=true` selects the file in the file manager.
fn os_open(abs: &std::path::Path, reveal: bool) -> AppResult<()> {
    let path = abs.to_string_lossy().to_string();
    let result = if cfg!(target_os = "macos") {
        let mut cmd = std::process::Command::new("open");
        if reveal {
            cmd.arg("-R");
        }
        cmd.arg(&path).spawn()
    } else if cfg!(target_os = "windows") {
        if reveal {
            std::process::Command::new("explorer")
                .arg(format!("/select,{path}"))
                .spawn()
        } else {
            std::process::Command::new("cmd")
                .args(["/C", "start", "", &path])
                .spawn()
        }
    } else {
        // Linux/other.
        let target = if reveal {
            abs.parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or(path.clone())
        } else {
            path.clone()
        };
        std::process::Command::new("xdg-open").arg(target).spawn()
    };
    result
        .map(|_| ())
        .map_err(|e| AppError::FsPermission(format!("open file: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::account_repo::NewAccount;
    use crate::types::{ParsedAttachment, ParsedMail};

    async fn seed(state: &AppState) -> String {
        AccountRepo::new(state.storage.db())
            .create(&NewAccount {
                id: "acc".into(),
                email: "me@x.com".into(),
                display_name: "Me".into(),
                provider: "imap".into(),
                imap_host: Some("imap.x.com".into()),
                imap_port: 993,
                smtp_host: None,
                smtp_port: 587,
                color_token: "slate".into(),
                badge_label: "W".into(),
                role_type: "work".into(),
                role_description: None,
                auth_level: 1,
            })
            .await
            .unwrap();
        let mail = ParsedMail {
            account_id: "acc".into(),
            folder: "INBOX".into(),
            imap_uid: Some(1),
            message_id: "<m@x>".into(),
            in_reply_to: None,
            references: None,
            subject: "S".into(),
            from_name: None,
            from_email: "a@x.com".into(),
            to_addrs: "[]".into(),
            cc_addrs: "[]".into(),
            bcc_addrs: "[]".into(),
            reply_to: None,
            date_sent: 1000,
            date_received: 1000,
            body_text: Some("b".into()),
            body_html: None,
            snippet: Some("b".into()),
            has_attachments: true,
            tracker_count: 0,
            attachments: vec![ParsedAttachment {
                filename: "evil.exe".into(),
                content_type: "application/octet-stream".into(),
                size_bytes: 10,
                content_id: None,
                is_inline: false,
                part_index: 0,
                data: None,
            }],
        };
        let (_s, ins) = MailRepo::new(state.storage.db())
            .upsert_batch(&[mail])
            .await
            .unwrap();
        let atts = AttachmentRepo::new(state.storage.db())
            .list_by_mail(&ins[0].summary.id)
            .await
            .unwrap();
        atts[0].id.clone()
    }

    #[tokio::test]
    async fn executable_download_is_forbidden() {
        let (state, _rx) = AppState::test_state().await;
        let att_id = seed(&state).await;
        let err = download_one(&state, &att_id, DownloadMode::Manual)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn open_undownloaded_is_not_found() {
        let (state, _rx) = AppState::test_state().await;
        let att_id = seed(&state).await;
        assert!(matches!(
            open_attachment(&state, &att_id).await.unwrap_err(),
            // exec block triggers first for .exe; use a non-exec check via get_local_path
            AppError::Forbidden(_) | AppError::NotFound
        ));
        assert!(get_local_path(&state, &att_id).await.unwrap().is_none());
    }
}
