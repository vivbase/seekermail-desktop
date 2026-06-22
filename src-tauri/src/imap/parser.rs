//! MIME parse worker (T023).
//!
//! Consumes `RawMail` from the ingest channel, parses it, runs the B1 sanitiser
//! (T027) over the HTML body, and persists via `MailRepo::upsert_batch`. Each new
//! mail emits `mail:new` and triggers auto attachment downloads (T025).
//!
//! Robustness choices:
//! * Body decode (charset/MIME/quoted-printable) uses `mail-parser` — the part of
//!   the API that is most stable.
//! * Address + reference headers are read from a tiny raw-header scanner so the
//!   pipeline does not depend on `mail-parser`'s address value types. (Documented
//!   deviation: lower fidelity for display names, but threading keys —
//!   Message-ID / In-Reply-To / References — are exact.)
//! * A message that fails to parse is logged (id + uid only, never content) and
//!   skipped — one bad mail never stalls the pipeline (F_A4 §4.5).

use std::collections::HashMap;

use mail_parser::{MessageParser, MimeHeaders};
use once_cell::sync::Lazy;
use regex::Regex;
use tokio::sync::mpsc;

use crate::sanitize::Sanitizer;
use crate::state::AppState;
use crate::storage::MailRepo;
use crate::types::{ParsedAttachment, ParsedMail, RawMail};
use crate::util::{normalize_email, now_unix, truncate_chars};

static EMAIL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}").unwrap());

/// Spawn the long-running parse worker. Returns its `JoinHandle`.
pub fn spawn_parse_worker(
    mut rx: mpsc::Receiver<RawMail>,
    state: AppState,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(raw) = rx.recv().await {
            if let Err(e) = handle_one(&state, &raw).await {
                // Boundary log — identifiers only, never body content (09 §5).
                tracing::warn!(
                    account_id = %raw.account_id,
                    imap_uid = raw.imap_uid,
                    error = %e,
                    "parse/persist failed; skipping mail"
                );
            }
        }
        tracing::info!(event = "parse_worker_stopped", "ingest channel closed");
    })
}

async fn handle_one(state: &AppState, raw: &RawMail) -> crate::error::AppResult<()> {
    let Some(mail) = parse_raw(raw, &state.sanitizer) else {
        tracing::warn!(account_id = %raw.account_id, imap_uid = raw.imap_uid, "unparsable mail skipped");
        return Ok(());
    };
    let had_attachments = mail.has_attachments;
    let account_id = mail.account_id.clone();
    let folder = mail.folder.clone();
    // Capture the body before `mail` is moved so we can enqueue it for embedding.
    let body_text = mail.body_text.clone();

    let (_stats, inserted) = MailRepo::new(state.storage.db())
        .upsert_batch(&[mail])
        .await?;

    // Folder-aware ingest policy (analysis/43 §3, "Junk quarantine"):
    //  • INBOX        → index for GTE + run the E2/E3 auto-reply pipeline + notify.
    //  • SENT         → index for GTE/style + notify; never auto-reply to our own
    //                   sent mail, so the pipeline is skipped.
    //  • JUNK / TRASH → persist only: no GTE index, no auto-reply, no notification.
    //    (Their rows are also written `embedding_status='skipped'`, so the embedding
    //    catch-up can't pull them in either — defence in depth.)
    let index = matches!(folder.as_str(), "INBOX" | "SENT");
    let run_pipeline = folder == "INBOX";
    let notify = matches!(folder.as_str(), "INBOX" | "SENT");

    for ins in inserted {
        // B3 hot path: enqueue for vectorisation (T031). A full queue is non-fatal —
        // the mail stays `pending` and the worker's catch-up poll claims it later.
        if index {
            if let Some(job) =
                crate::embedding::queue::job_from_summary(&ins.summary, body_text.as_deref())
            {
                state.embed_queue.try_send(job);
            }
        }
        // E2/E3 AI pipeline hook (T082): one job per freshly INSERTED inbound mail.
        // Non-blocking; a full queue just skips automatic drafting for this mail.
        if run_pipeline {
            state
                .pipeline_queue
                .try_enqueue(crate::ai::pipeline::worker::E2PipelineJob {
                    mail_id: ins.summary.id.clone(),
                    account_id: account_id.clone(),
                });
        }
        if notify {
            state.events.mail_new(ins.summary);
        }
    }
    // A5 branch of the shared pipeline: queue document attachments (T025). INBOX
    // only — we don't chase attachments (or links) out of Junk.
    if had_attachments && folder == "INBOX" {
        crate::imap::attachment::trigger_auto(state, &account_id).await;
    }
    Ok(())
}

/// Parse one raw RFC-822 message into a [`ParsedMail`]. `None` on hard parse
/// failure. The HTML body is sanitised here so the persisted `body_html` is the
/// safe version (T027 §3).
pub fn parse_raw(raw: &RawMail, sanitizer: &Sanitizer) -> Option<ParsedMail> {
    let msg = MessageParser::default().parse(&raw.raw_bytes)?;

    let headers = scan_headers(&raw.raw_bytes);
    let subject = msg.subject().map(|s| s.to_string()).unwrap_or_default();
    let message_id = header_first(&headers, "message-id")
        .or_else(|| msg.message_id().map(|s| s.to_string()))
        .unwrap_or_else(|| format!("<{}-{}@seekermail.local>", raw.account_id, raw.imap_uid));
    let in_reply_to = header_first(&headers, "in-reply-to");
    let references = headers.get("references").cloned();

    let (from_name, from_email) =
        first_address(&headers, "from").unwrap_or_else(|| (None, "unknown@localhost".to_string()));
    let to_addrs = addresses_json(&headers, "to");
    let cc_addrs = addresses_json(&headers, "cc");

    let date_received = now_unix();
    let date_sent = msg
        .date()
        .map(|d| d.to_timestamp())
        .unwrap_or(date_received);

    // Body: prefer HTML (sanitised), fall back to plain text.
    let raw_html = msg.body_html(0).map(|c| c.into_owned());
    let plain = msg.body_text(0).map(|c| c.into_owned());

    let (body_html, body_text, tracker_count) = match raw_html {
        Some(html) => {
            let out = sanitizer.clean(&html);
            let text = if out.body_text.is_empty() {
                plain.clone().unwrap_or_default()
            } else {
                out.body_text
            };
            (Some(out.html), Some(text), out.tracker_count)
        }
        None => (None, plain.clone(), 0),
    };

    let snippet = body_text
        .as_deref()
        .map(|t| truncate_chars(t.trim(), 200))
        .filter(|s| !s.is_empty())
        .or_else(|| Some(truncate_chars(&subject, 200)));

    // Attachments: metadata only (bytes downloaded later, T025). `part_index` is
    // the attachment's ordinal position in this same `attachments()` iterator;
    // `fetch_part` re-parses the message and addresses the part by that index, so
    // the two must enumerate identically (they call the same parser on the same
    // bytes). The real `content_type` is read from the part's MIME headers and
    // only degrades to `application/octet-stream` when the header is truly absent.
    let mut attachments: Vec<ParsedAttachment> = Vec::new();
    for (idx, part) in msg.attachments().enumerate() {
        let filename = part
            .attachment_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "attachment.bin".to_string());
        let content_id = part.content_id().map(|s| s.to_string());
        let content_type = part
            .content_type()
            .map(|ct| match ct.subtype() {
                Some(sub) => format!("{}/{}", ct.ctype(), sub),
                None => ct.ctype().to_string(),
            })
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let size_bytes = part.contents().len() as u64;
        attachments.push(ParsedAttachment {
            filename,
            content_type,
            size_bytes,
            is_inline: content_id.is_some(),
            content_id,
            part_index: idx as u32,
            data: None,
        });
    }
    let has_attachments = attachments.iter().any(|a| !a.is_inline);

    Some(ParsedMail {
        account_id: raw.account_id.clone(),
        folder: raw.folder.clone(),
        imap_uid: Some(raw.imap_uid),
        message_id,
        in_reply_to,
        references,
        subject,
        from_name,
        from_email: normalize_email(&from_email),
        to_addrs,
        cc_addrs,
        bcc_addrs: "[]".to_string(),
        reply_to: None,
        date_sent,
        date_received,
        body_text,
        body_html,
        snippet,
        has_attachments,
        tracker_count,
        attachments,
    })
}

/// Parse the header block (up to the first blank line) into a lowercased map,
/// unfolding continuation lines. Last value wins for duplicate header names.
fn scan_headers(raw: &[u8]) -> HashMap<String, String> {
    let text = String::from_utf8_lossy(raw);
    let header_block = text.split("\r\n\r\n").next().unwrap_or(&text);
    let header_block = header_block.split("\n\n").next().unwrap_or(header_block);

    let mut map: HashMap<String, String> = HashMap::new();
    let mut cur_key: Option<String> = None;
    let mut cur_val = String::new();
    for line in header_block.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            // Folded continuation.
            cur_val.push(' ');
            cur_val.push_str(line.trim());
            continue;
        }
        if let Some(k) = cur_key.take() {
            map.insert(k, cur_val.trim().to_string());
            cur_val.clear();
        }
        if let Some((k, v)) = line.split_once(':') {
            cur_key = Some(k.trim().to_lowercase());
            cur_val.push_str(v.trim());
        }
    }
    if let Some(k) = cur_key.take() {
        map.insert(k, cur_val.trim().to_string());
    }
    map
}

fn header_first(headers: &HashMap<String, String>, key: &str) -> Option<String> {
    headers
        .get(key)
        .map(|v| v.split_whitespace().next().unwrap_or(v).to_string())
}

/// First (name, email) for an address header.
fn first_address(headers: &HashMap<String, String>, key: &str) -> Option<(Option<String>, String)> {
    let raw = headers.get(key)?;
    let email = EMAIL_RE.find(raw)?.as_str().to_string();
    // Display name: text before '<' if present.
    let name = raw
        .split('<')
        .next()
        .map(|n| n.trim().trim_matches('"').to_string())
        .filter(|n| !n.is_empty() && !n.contains('@'));
    Some((name, email))
}

/// JSON array of `{name,email}` for an address header (best-effort extraction).
fn addresses_json(headers: &HashMap<String, String>, key: &str) -> String {
    let Some(raw) = headers.get(key) else {
        return "[]".to_string();
    };
    let entries: Vec<serde_json::Value> = EMAIL_RE
        .find_iter(raw)
        .map(|m| serde_json::json!({ "name": "", "email": m.as_str().to_lowercase() }))
        .collect();
    serde_json::to_string(&entries).unwrap_or_else(|_| "[]".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(bytes: &str) -> RawMail {
        RawMail {
            account_id: "acc".into(),
            folder: "INBOX".into(),
            imap_uid: 42,
            raw_bytes: bytes.as_bytes().to_vec(),
        }
    }

    #[test]
    fn parses_plain_message() {
        let eml = "From: Alice <alice@example.com>\r\nTo: bob@example.com\r\nSubject: Hi there\r\nMessage-ID: <m1@example.com>\r\nDate: Mon, 1 Jan 2024 10:00:00 +0000\r\n\r\nHello body text.";
        let m = parse_raw(&raw(eml), &Sanitizer::new()).unwrap();
        assert_eq!(m.from_email, "alice@example.com");
        assert_eq!(m.subject, "Hi there");
        assert_eq!(m.message_id, "<m1@example.com>");
        assert!(m.body_text.unwrap().contains("Hello body"));
    }

    #[test]
    fn reply_headers_extracted() {
        let eml = "From: a@x.com\r\nSubject: Re: Hi\r\nMessage-ID: <m2@x>\r\nIn-Reply-To: <m1@x>\r\nReferences: <m0@x> <m1@x>\r\n\r\nbody";
        let m = parse_raw(&raw(eml), &Sanitizer::new()).unwrap();
        assert_eq!(m.in_reply_to.as_deref(), Some("<m1@x>"));
        assert_eq!(m.references.as_deref(), Some("<m0@x> <m1@x>"));
    }

    #[test]
    fn html_body_is_sanitised() {
        let eml = "From: a@x.com\r\nSubject: S\r\nMessage-ID: <m@x>\r\nContent-Type: text/html\r\n\r\n<p>Hi</p><script>alert(1)</script>";
        let m = parse_raw(&raw(eml), &Sanitizer::new()).unwrap();
        let html = m.body_html.unwrap();
        assert!(html.contains("Hi"));
        assert!(!html.contains("script"));
    }

    #[test]
    fn header_folding_is_unfolded() {
        let eml =
            "From: a@x.com\r\nSubject: A very\r\n long subject\r\nMessage-ID: <m@x>\r\n\r\nbody";
        let m = parse_raw(&raw(eml), &Sanitizer::new()).unwrap();
        assert!(m.subject.contains("long subject") || m.subject == "A very long subject");
    }

    /// A multipart message with one PDF attachment: the real `content_type` is
    /// preserved (not the old hardcoded `octet-stream`) and the part index is 0.
    const MULTIPART_EML: &str = "From: a@x.com\r\n\
Subject: With file\r\n\
Message-ID: <m@x>\r\n\
Content-Type: multipart/mixed; boundary=\"b\"\r\n\
\r\n\
--b\r\n\
Content-Type: text/plain\r\n\
\r\n\
Hello body\r\n\
--b\r\n\
Content-Type: application/pdf; name=\"report.pdf\"\r\n\
Content-Disposition: attachment; filename=\"report.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
SGVsbG8=\r\n\
--b--\r\n";

    #[test]
    fn attachment_content_type_and_part_index_are_real() {
        let m = parse_raw(&raw(MULTIPART_EML), &Sanitizer::new()).unwrap();
        assert_eq!(m.attachments.len(), 1);
        let att = &m.attachments[0];
        assert_eq!(att.content_type, "application/pdf");
        assert_eq!(att.filename, "report.pdf");
        assert_eq!(att.part_index, 0);
        assert!(!att.is_inline);
        // `size_bytes` is the decoded length (base64 "SGVsbG8=" → "Hello" = 5).
        assert_eq!(att.size_bytes, 5);
    }

    #[test]
    fn nth_attachment_round_trips_bytes() {
        // Mirrors `LiveImapSession::fetch_part`: parse the full message and slice
        // the attachment at the stored part index. This is the exact mechanism the
        // deferred download relies on, validated here without a live IMAP server.
        let msg = MessageParser::default()
            .parse(MULTIPART_EML.as_bytes())
            .unwrap();
        let part = msg.attachments().nth(0).expect("one attachment at index 0");
        assert_eq!(part.contents(), b"Hello");
    }
}
