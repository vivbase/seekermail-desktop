//! JSON Lines + MANIFEST.json writers for the export task (T052).
//!
//! One `.jsonl` file per account (one mail object per line) plus a single
//! `MANIFEST.json` describing the bundle. Field set follows the card:
//! `id, from, to, subject, date, body_text?, has_attachment, tags`.

use std::io::Write;

use serde::Serialize;

use crate::error::{AppError, AppResult};

use super::{parse_recipients, ExportRow, Recipient};

/// `MANIFEST.json` format version — bump on breaking layout changes.
pub const MANIFEST_FORMAT_VERSION: u32 = 1;

#[derive(Serialize)]
struct JsonMail<'a> {
    id: &'a str,
    from: Recipient,
    to: Vec<Recipient>,
    subject: &'a str,
    date: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_text: Option<&'a str>,
    has_attachment: bool,
    /// Reserved — tag support arrives with the v1.0 metadata cards.
    tags: Vec<String>,
}

/// Append one mail as a single JSON line.
pub fn write_mail<W: Write>(out: &mut W, row: &ExportRow, include_body: bool) -> AppResult<()> {
    let mail = JsonMail {
        id: &row.id,
        from: Recipient {
            name: row.from_name.clone(),
            email: row.from_email.clone(),
        },
        to: parse_recipients(&row.to_addrs),
        subject: &row.subject,
        date: row.date_sent,
        body_text: if include_body {
            row.body_text.as_deref()
        } else {
            None
        },
        has_attachment: row.has_attachments != 0,
        tags: Vec::new(),
    };
    let line = serde_json::to_string(&mail)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("jsonl serialize: {e}")))?;
    out.write_all(line.as_bytes())
        .and_then(|()| out.write_all(b"\n"))
        .map_err(|e| AppError::Internal(anyhow::anyhow!("jsonl write: {e}")))
}

/// Everything MANIFEST.json records about one export run.
#[derive(Serialize)]
pub struct Manifest {
    pub format_version: u32,
    /// "mbox" | "json"
    pub format: String,
    /// RFC 3339 UTC export timestamp.
    pub exported_at: String,
    pub account_ids: Vec<String>,
    pub mail_count: u64,
    pub include_body: bool,
    pub include_attachments: bool,
    /// Attachments referenced but not locally downloaded → not in the bundle.
    pub skipped_attachments: u64,
    /// Relative file names inside the bundle.
    pub files: Vec<String>,
}

pub fn write_manifest<W: Write>(out: &mut W, manifest: &Manifest) -> AppResult<()> {
    let body = serde_json::to_string_pretty(manifest)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("manifest serialize: {e}")))?;
    out.write_all(body.as_bytes())
        .map_err(|e| AppError::Internal(anyhow::anyhow!("manifest write: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonl_line_is_valid_json_and_respects_include_body() {
        let row = ExportRow {
            id: "m-1".into(),
            message_id: "<m1@example.com>".into(),
            in_reply_to: None,
            subject: "Hello".into(),
            from_name: None,
            from_email: "a@b.c".into(),
            to_addrs: r#"[{"name":null,"email":"d@e.f"}]"#.into(),
            cc_addrs: "[]".into(),
            date_sent: 100,
            body_text: Some("secret body".into()),
            has_attachments: 1,
        };

        let mut with_body: Vec<u8> = Vec::new();
        write_mail(&mut with_body, &row, true).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&with_body).unwrap();
        assert_eq!(v["id"], "m-1");
        assert_eq!(v["body_text"], "secret body");
        assert_eq!(v["has_attachment"], true);
        assert_eq!(v["to"][0]["email"], "d@e.f");

        let mut without: Vec<u8> = Vec::new();
        write_mail(&mut without, &row, false).unwrap();
        let v2: serde_json::Value = serde_json::from_slice(&without).unwrap();
        assert!(v2.get("body_text").is_none());
    }

    #[test]
    fn manifest_serializes_with_count_and_version() {
        let m = Manifest {
            format_version: MANIFEST_FORMAT_VERSION,
            format: "json".into(),
            exported_at: "2026-06-12T00:00:00Z".into(),
            account_ids: vec!["a1".into()],
            mail_count: 10,
            include_body: true,
            include_attachments: false,
            skipped_attachments: 0,
            files: vec!["account-a1.jsonl".into()],
        };
        let mut out: Vec<u8> = Vec::new();
        write_manifest(&mut out, &m).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["mail_count"], 10);
        assert_eq!(v["format_version"], 1);
    }
}
