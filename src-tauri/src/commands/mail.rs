//! Mail reading-view commands — tracker info + remote-image allow (T029) — plus
//! compose send + cancel (T043).

use tauri::State;

use crate::error::{AppError, AppResult, IpcError};
use crate::send;
use crate::state::AppState;
use crate::storage::{AttachmentRepo, MailRepo, OutboundOpKind, OutboundOpRepo, SettingRepo};
use crate::types::{
    CancelSendResult, ImageAllowScope, InlineImage, ListMailsParams, ListThreadsParams, MailDetail,
    MailSummary, PageResult, RemoteImage, SendMailParams, SendMailResult, Thread, TrackerInfo,
};

/// Read tracker status for one mail + whether its sender's images are allowed.
#[tauri::command]
pub async fn get_tracker_info(
    state: State<'_, AppState>,
    mail_id: String,
) -> Result<TrackerInfo, IpcError> {
    let (sender_email, blocked, tracker_count) = MailRepo::new(state.storage.db())
        .tracker_row(&mail_id)
        .await
        .map_err(IpcError::from)?;
    let images_allowed = SettingRepo::new(state.storage.db())
        .is_sender_image_allowed(&sender_email)
        .await
        .map_err(IpcError::from)?;
    Ok(TrackerInfo {
        blocked,
        tracker_count,
        images_allowed,
        sender_email,
    })
}

/// Persist a remote-image allow decision. `ThisMessage` is handled entirely on the
/// frontend (a one-shot DOM swap); only `AlwaysSender` persists (T029 §3).
#[tauri::command]
pub async fn allow_remote_images(
    state: State<'_, AppState>,
    mail_id: String,
    scope: ImageAllowScope,
) -> Result<(), IpcError> {
    match scope {
        ImageAllowScope::ThisMessage => {
            tracing::debug!(mail_id = %mail_id, "remote images allowed for this message (frontend DOM)");
        }
        ImageAllowScope::AlwaysSender { sender_email } => {
            SettingRepo::new(state.storage.db())
                .add_image_allow_sender(&sender_email)
                .await
                .map_err(IpcError::from)?;
        }
    }
    Ok(())
}

// ── Inline (cid:) image resolution (F_G3 §4.1) ───────────────────────────────
// Inline images ship inside the message but are stored metadata-only at ingest
// (T023/T025): the bytes are fetched on demand here over the same transport seam
// as attachment download, then returned base64-encoded so the reading view can
// swap `<img src="cid:…">` to a `data:` URI. They never leave the device and are
// not gated behind "load images" — only REMOTE images are (privacy default).

/// Resolve a mail's inline (`cid:`) images to bytes for the reading view.
///
/// Per-image failures are logged and skipped so one missing part never blanks
/// the whole body. The only network traffic is re-fetching message parts the
/// mail already carried; nothing is sent to a third party.
#[tauri::command]
pub async fn get_inline_images(
    state: State<'_, AppState>,
    mail_id: String,
) -> Result<Vec<InlineImage>, IpcError> {
    use base64::Engine as _;

    let atts = AttachmentRepo::new(state.storage.db())
        .list_by_mail(&mail_id)
        .await
        .map_err(IpcError::from)?;

    let mut out = Vec::new();
    for att in atts.into_iter().filter(|a| a.is_inline) {
        let Some(cid) = att.content_id.as_deref().map(normalize_cid) else {
            continue;
        };
        if cid.is_empty() {
            continue;
        }
        // Ensure the bytes are on disk: reuse the already-downloaded blob, else
        // fetch the part once (Manual lane — the user is looking at the mail).
        let rel = if att.downloaded {
            match att.local_path.clone() {
                Some(path) => path,
                None => continue,
            }
        } else {
            match crate::imap::attachment::download_one(
                state.inner(),
                &att.id,
                crate::imap::attachment::DownloadMode::Manual,
            )
            .await
            {
                Ok(path) => path,
                Err(e) => {
                    tracing::debug!(error = %e, "inline image fetch skipped");
                    continue;
                }
            }
        };
        let bytes = match state.storage.blobs().read_attachment(&rel).await {
            Ok(b) => b,
            Err(e) => {
                tracing::debug!(error = %e, "inline image read skipped");
                continue;
            }
        };
        out.push(InlineImage {
            content_id: cid,
            mime: image_mime(&att.content_type, &bytes),
            data_base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
        });
    }
    Ok(out)
}

/// Normalise a Content-ID for matching against a `cid:` URL: strip surrounding
/// angle brackets and whitespace (RFC 2392 references the id without brackets).
fn normalize_cid(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_string()
}

/// Best-effort image MIME for an inline part: trust a real `image/*`
/// Content-Type, otherwise sniff the leading magic bytes (some senders label
/// inline images `application/octet-stream`). Falls back to PNG.
fn image_mime(content_type: &str, bytes: &[u8]) -> String {
    let ct = content_type.trim().to_ascii_lowercase();
    if ct.starts_with("image/") {
        return ct;
    }
    sniff_image_mime(bytes).unwrap_or("image/png").to_string()
}

/// Recognise the common raster image signatures. SVG is intentionally NOT
/// sniffed — only trusted when the part's own Content-Type already says so.
fn sniff_image_mime(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        return Some("image/png");
    }
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if b.starts_with(b"GIF8") {
        return Some("image/gif");
    }
    if b.starts_with(b"BM") {
        return Some("image/bmp");
    }
    if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

// ── Privacy-hardened remote-image fetch (F_B2 §4.3 as a LOCAL fetch) ──────────
// Remote images stay blocked by default; when the user (or an allow-listed
// sender) loads them, the bytes are fetched HERE — not by the webview — with no
// cookies, no Referer, and no User-Agent, so the origin gets the bare minimum.
// The webview only ever sees a `data:` URI (CSP `img-src data:`).

/// Cap on a single remote image we will buffer + base64 into the webview.
const MAX_REMOTE_IMAGE_BYTES: usize = 16 * 1024 * 1024;

/// Fetch one remote image through the backend and return it base64-encoded for a
/// `data:` URI swap. Only `image/*` responses under the size cap are accepted,
/// redirects are capped and re-checked, and obviously-internal hosts are refused
/// so this can't double as an SSRF probe of the user's own network.
#[tauri::command]
pub async fn fetch_remote_image(url: String) -> Result<RemoteImage, IpcError> {
    fetch_remote_image_inner(url).await.map_err(IpcError::from)
}

async fn fetch_remote_image_inner(url: String) -> AppResult<RemoteImage> {
    use base64::Engine as _;

    let parsed = validate_remote_image_url(&url)?;

    // Hardened client: no cookie store (default), empty User-Agent, capped +
    // re-validated redirects, bounded timeouts. rustls only (packaging, 05).
    let client = reqwest::Client::builder()
        .user_agent("")
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 3 {
                return attempt.error("too many redirects");
            }
            match attempt.url().host_str() {
                Some(h) if is_blocked_host(h) => attempt.error("redirect to internal host"),
                _ => attempt.follow(),
            }
        }))
        .connect_timeout(std::time::Duration::from_secs(8))
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("http client init: {e}")))?;

    let resp = client
        .get(parsed)
        .header(reqwest::header::ACCEPT, "image/*")
        .send()
        .await
        .map_err(|_| AppError::ImapConnection("remote image fetch failed".into()))?;

    if !resp.status().is_success() {
        return Err(AppError::NotFound);
    }

    let mime = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            s.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
        .unwrap_or_default();
    if !mime.starts_with("image/") {
        return Err(AppError::Validation(
            "remote response is not an image".into(),
        ));
    }
    if let Some(len) = resp.content_length() {
        if len as usize > MAX_REMOTE_IMAGE_BYTES {
            return Err(AppError::Validation("remote image too large".into()));
        }
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|_| AppError::ImapConnection("remote image body read failed".into()))?;
    if bytes.len() > MAX_REMOTE_IMAGE_BYTES {
        return Err(AppError::Validation("remote image too large".into()));
    }

    Ok(RemoteImage {
        mime,
        data_base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
    })
}

/// Validate a remote-image URL: http(s) only, with a host that isn't obviously
/// internal. Returns the parsed URL ready to fetch.
fn validate_remote_image_url(raw: &str) -> AppResult<reqwest::Url> {
    let url =
        reqwest::Url::parse(raw).map_err(|_| AppError::Validation("invalid image url".into()))?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err(AppError::Validation("unsupported url scheme".into())),
    }
    let host = url
        .host_str()
        .ok_or_else(|| AppError::Validation("missing host".into()))?;
    if is_blocked_host(host) {
        return Err(AppError::Forbidden("blocked internal host".into()));
    }
    Ok(url)
}

/// Reject obviously-internal hosts so the fetch can't probe the user's own
/// network. Literal-IP / localhost only — a hostname that resolves to a private
/// address via DNS is NOT covered here (documented follow-up; the request is
/// user-initiated and the bytes return only to the same user).
fn is_blocked_host(host: &str) -> bool {
    let h = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    if h == "localhost" || h.ends_with(".localhost") || h.is_empty() {
        return true;
    }
    if let Ok(ip) = h.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(v4) => {
                v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_unspecified()
                    || v4.is_broadcast()
                    || v4.octets()[0] == 0
            }
            std::net::IpAddr::V6(v6) => {
                v6.is_loopback()
                    || v6.is_unspecified()
                    || (v6.segments()[0] & 0xfe00) == 0xfc00 // unique-local fc00::/7
                    || (v6.segments()[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
            }
        };
    }
    false
}

/// Queue a message for send behind the 10-second cancel window (T043). Returns
/// immediately with the pending id (for `cancel_send`) and the message id.
#[tauri::command]
pub async fn send_mail(
    state: State<'_, AppState>,
    params: SendMailParams,
) -> Result<SendMailResult, IpcError> {
    send::schedule_send(&state, params)
        .await
        .map_err(IpcError::from)
}

/// Cancel a pending send within its window. Two-path lookup (T043 + T085 §6):
/// `pendingId` is first tried against the in-memory SMTP queue (a T043
/// `schedule_send` pending id, 10 s window); when absent there it is treated
/// as an E3 `ai_drafts.id` inside its 30 s `send_after` window — a successful
/// E3 cancel clears `send_after`, keeps the draft `pending`, audits
/// `auto_send_cancelled`, and re-emits `draft:ready` so the draft returns to
/// the Pending review queue. `cancelled=false` if both windows have elapsed
/// or the id is unknown.
#[tauri::command]
pub async fn cancel_send(
    state: State<'_, AppState>,
    pending_id: String,
) -> Result<CancelSendResult, IpcError> {
    let direct = send::cancel_send(&state, &pending_id);
    if direct.cancelled {
        return Ok(direct);
    }
    crate::ai::pipeline::e3_send_queue::cancel_pending_auto_send(&state, &pending_id)
        .await
        .map_err(IpcError::from)
}

// ── Mail-list read backend (G2/G3) ───────────────────────────────────────────
// The L0 stream (`ThreadList`) and the reading view consume these. Without them
// the webview's `invoke("list_threads" | "list_mails" | "get_mail")` calls reject
// and every mail surface renders empty.

/// Paginated thread list for the folded L0 stream (G2).
#[tauri::command]
pub async fn list_threads(
    state: State<'_, AppState>,
    params: ListThreadsParams,
) -> Result<PageResult<Thread>, IpcError> {
    MailRepo::new(state.storage.db())
        .list_threads(&params)
        .await
        .map_err(IpcError::from)
}

/// Paginated flat mail list — unread / processed / all-mail routes (G3).
#[tauri::command]
pub async fn list_mails(
    state: State<'_, AppState>,
    params: ListMailsParams,
) -> Result<PageResult<MailSummary>, IpcError> {
    MailRepo::new(state.storage.db())
        .list_mails(&params)
        .await
        .map_err(IpcError::from)
}

/// Full mail detail for the reading view (G3).
#[tauri::command]
pub async fn get_mail(state: State<'_, AppState>, mail_id: String) -> Result<MailDetail, IpcError> {
    MailRepo::new(state.storage.db())
        .get_mail(&mail_id)
        .await
        .map_err(IpcError::from)
}

/// Mark a mail read/unread (drives `mail:updated`) and sync `\Seen` to the server.
#[tauri::command]
pub async fn set_mail_read(
    state: State<'_, AppState>,
    mail_id: String,
    is_read: bool,
) -> Result<(), IpcError> {
    MailRepo::new(state.storage.db())
        .set_read(&mail_id, is_read)
        .await
        .map_err(IpcError::from)?;
    let kind = if is_read {
        OutboundOpKind::MarkSeen
    } else {
        OutboundOpKind::MarkUnseen
    };
    enqueue_writeback(&state, &mail_id, kind).await;
    Ok(())
}

/// Star / unstar a mail and sync `\Flagged` to the server.
#[tauri::command]
pub async fn set_mail_starred(
    state: State<'_, AppState>,
    mail_id: String,
    is_starred: bool,
) -> Result<(), IpcError> {
    MailRepo::new(state.storage.db())
        .set_starred(&mail_id, is_starred)
        .await
        .map_err(IpcError::from)?;
    let kind = if is_starred {
        OutboundOpKind::Flag
    } else {
        OutboundOpKind::Unflag
    };
    enqueue_writeback(&state, &mail_id, kind).await;
    Ok(())
}

/// Queue a write-back for one mail and kick a prompt drain. Best-effort: the local
/// update already succeeded, so a sync hiccup never fails the command. The drain
/// worker resolves the live server folder name (the local `folder` tag is not it)
/// and skips anything it can't map. Mail composed locally (no `imap_uid`) has
/// nothing to write back.
async fn enqueue_writeback(state: &State<'_, AppState>, mail_id: &str, kind: OutboundOpKind) {
    let repo = MailRepo::new(state.storage.db());
    if let Ok(Some((account_id, folder, Some(uid)))) = repo.imap_coords(mail_id).await {
        let _ = OutboundOpRepo::new(state.storage.db())
            .enqueue(&account_id, &folder, uid, kind)
            .await;
        crate::imap::outbound::spawn_drain(state.inner().clone(), account_id);
    }
}

/// Archive a mail (removes it from the active streams) and archive it server-side
/// (move to the Archive folder, or out of the INBOX label on Gmail).
#[tauri::command]
pub async fn archive_mail(state: State<'_, AppState>, mail_id: String) -> Result<(), IpcError> {
    MailRepo::new(state.storage.db())
        .set_archived(&mail_id, true)
        .await
        .map_err(IpcError::from)?;
    enqueue_writeback(&state, &mail_id, OutboundOpKind::Archive).await;
    Ok(())
}

/// Soft-delete a mail (local `is_deleted = 1`) and move it to the server Trash.
#[tauri::command]
pub async fn delete_mail(state: State<'_, AppState>, mail_id: String) -> Result<(), IpcError> {
    MailRepo::new(state.storage.db())
        .set_deleted(&mail_id, true)
        .await
        .map_err(IpcError::from)?;
    enqueue_writeback(&state, &mail_id, OutboundOpKind::Trash).await;
    Ok(())
}

/// Mark a mail as spam: drop it from the active stream locally and move it to the
/// server Junk folder. (Recovering a false positive — "not spam" back to the
/// Inbox — is a follow-up.)
#[tauri::command]
pub async fn set_mail_spam(state: State<'_, AppState>, mail_id: String) -> Result<(), IpcError> {
    MailRepo::new(state.storage.db())
        .mark_spam(&mail_id)
        .await
        .map_err(IpcError::from)?;
    enqueue_writeback(&state, &mail_id, OutboundOpKind::MarkSpam).await;
    Ok(())
}

/// Restore a trashed (or archived) mail to the Inbox (analysis/44 §5) and move it
/// back on the server. The message's current server location is captured *before*
/// the local restore rewrites its folder/UID, so the write-back targets the right
/// UID; the move op shares the relocation family, so an undo issued before the
/// original Trash move drained simply cancels it (no round-trip). Best-effort: the
/// local restore already stands even if the server move is deferred.
#[tauri::command]
pub async fn restore_mail(state: State<'_, AppState>, mail_id: String) -> Result<(), IpcError> {
    let repo = MailRepo::new(state.storage.db());
    let coords = repo.imap_coords(&mail_id).await.map_err(IpcError::from)?;
    repo.restore_to_inbox(&mail_id)
        .await
        .map_err(IpcError::from)?;
    if let Some((account_id, folder, Some(uid))) = coords {
        let _ = OutboundOpRepo::new(state.storage.db())
            .enqueue(&account_id, &folder, uid, OutboundOpKind::Restore)
            .await;
        crate::imap::outbound::spawn_drain(state.inner().clone(), account_id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_cid_strips_brackets_and_space() {
        assert_eq!(normalize_cid("<logo@seeker>"), "logo@seeker");
        assert_eq!(normalize_cid("  <part1.abc>  "), "part1.abc");
        assert_eq!(normalize_cid("plain-id"), "plain-id");
        assert_eq!(normalize_cid("<>"), "");
    }

    #[test]
    fn image_mime_trusts_real_type_then_sniffs() {
        // A declared image type wins (lower-cased).
        assert_eq!(image_mime("IMAGE/PNG", &[]), "image/png");
        // octet-stream falls back to a signature sniff.
        let png = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(image_mime("application/octet-stream", &png), "image/png");
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0];
        assert_eq!(image_mime("application/octet-stream", &jpeg), "image/jpeg");
        // Unknown bytes default to PNG rather than leaking octet-stream.
        assert_eq!(
            image_mime("application/octet-stream", b"not-an-image"),
            "image/png"
        );
    }

    #[test]
    fn sniff_recognises_common_rasters() {
        assert_eq!(sniff_image_mime(b"GIF89a..."), Some("image/gif"));
        assert_eq!(sniff_image_mime(b"BM\x00\x00"), Some("image/bmp"));
        let webp = b"RIFF\x00\x00\x00\x00WEBPVP8 ";
        assert_eq!(sniff_image_mime(webp), Some("image/webp"));
        assert_eq!(sniff_image_mime(b"<svg></svg>"), None); // SVG never sniffed
    }

    #[test]
    fn url_validation_accepts_public_https_only() {
        assert!(validate_remote_image_url("https://cdn.example.com/a.png").is_ok());
        assert!(validate_remote_image_url("http://images.example.com/x.jpg").is_ok());
        // Non-web schemes are refused.
        assert!(validate_remote_image_url("file:///etc/passwd").is_err());
        assert!(validate_remote_image_url("data:image/png;base64,AAAA").is_err());
        assert!(validate_remote_image_url("not a url").is_err());
    }

    #[test]
    fn blocked_hosts_cover_localhost_and_private_ranges() {
        for h in [
            "localhost",
            "app.localhost",
            "127.0.0.1",
            "10.0.0.5",
            "192.168.1.1",
            "172.16.4.2",
            "169.254.10.1",
            "0.0.0.0",
            "::1",
            "[::1]",
            "fe80::1",
            "fc00::1",
        ] {
            assert!(is_blocked_host(h), "{h} should be blocked");
        }
        for h in ["example.com", "8.8.8.8", "cdn.shopify.com", "1.1.1.1"] {
            assert!(!is_blocked_host(h), "{h} should be allowed");
        }
    }

    #[test]
    fn url_validation_rejects_internal_hosts() {
        assert!(validate_remote_image_url("http://localhost:8080/x.png").is_err());
        assert!(validate_remote_image_url("http://127.0.0.1/x.png").is_err());
        assert!(validate_remote_image_url("https://192.168.0.10/logo.gif").is_err());
    }
}
