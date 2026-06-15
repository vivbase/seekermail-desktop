//! mbox (RFC 4155) writer for the export task (T052).
//!
//! Each message starts with a `From <sender> <asctime>` separator line; headers
//! are reconstructed from DB columns; lines in the body that could be mistaken
//! for a separator are `>`-escaped (`From ` and `>+From ` forms both get one
//! more `>`). Output uses CRLF line endings per the card spec. Credentials are
//! structurally absent — this module only ever sees `mails` rows.

use std::io::Write;

use chrono::{TimeZone, Utc};

use crate::error::{AppError, AppResult};

use super::{recipients_header, ExportRow};

/// asctime-style timestamp for the `From ` separator line.
fn asctime(ts: i64) -> String {
    match Utc.timestamp_opt(ts, 0) {
        chrono::LocalResult::Single(dt) => dt.format("%a %b %e %H:%M:%S %Y").to_string(),
        _ => Utc
            .timestamp_opt(0, 0)
            .unwrap()
            .format("%a %b %e %H:%M:%S %Y")
            .to_string(),
    }
}

/// RFC 2822 date header value.
fn rfc2822(ts: i64) -> String {
    match Utc.timestamp_opt(ts, 0) {
        chrono::LocalResult::Single(dt) => dt.format("%a, %d %b %Y %H:%M:%S +0000").to_string(),
        _ => String::new(),
    }
}

/// `>`-escape a body line if it would shadow the mbox separator.
fn escape_from_line(line: &str) -> String {
    let stripped = line.trim_start_matches('>');
    if stripped.starts_with("From ") {
        format!(">{line}")
    } else {
        line.to_string()
    }
}

/// Append one mail to an open mbox stream.
pub fn write_mail<W: Write>(out: &mut W, row: &ExportRow) -> AppResult<()> {
    let mut buf = String::new();

    // Separator line.
    buf.push_str(&format!(
        "From {} {}\r\n",
        row.from_email,
        asctime(row.date_sent)
    ));

    // Reconstructed headers.
    let from_header = match &row.from_name {
        Some(name) if !name.is_empty() => format!("{} <{}>", name, row.from_email),
        _ => row.from_email.clone(),
    };
    buf.push_str(&format!("From: {from_header}\r\n"));
    let to = recipients_header(&row.to_addrs);
    if !to.is_empty() {
        buf.push_str(&format!("To: {to}\r\n"));
    }
    let cc = recipients_header(&row.cc_addrs);
    if !cc.is_empty() {
        buf.push_str(&format!("Cc: {cc}\r\n"));
    }
    buf.push_str(&format!(
        "Subject: {}\r\n",
        row.subject.replace(['\r', '\n'], " ")
    ));
    buf.push_str(&format!("Date: {}\r\n", rfc2822(row.date_sent)));
    buf.push_str(&format!("Message-ID: {}\r\n", row.message_id));
    if let Some(irt) = &row.in_reply_to {
        if !irt.is_empty() {
            buf.push_str(&format!("In-Reply-To: {irt}\r\n"));
        }
    }
    buf.push_str("\r\n");

    // Body with `>From` escaping.
    if let Some(body) = &row.body_text {
        for line in body.lines() {
            buf.push_str(&escape_from_line(line));
            buf.push_str("\r\n");
        }
    }
    buf.push_str("\r\n");

    out.write_all(buf.as_bytes())
        .map_err(|e| AppError::Internal(anyhow::anyhow!("mbox write: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> ExportRow {
        ExportRow {
            id: "m-1".into(),
            message_id: "<m1@example.com>".into(),
            in_reply_to: None,
            subject: "Invoice for May".into(),
            from_name: Some("Alice".into()),
            from_email: "alice@example.com".into(),
            to_addrs: r#"[{"name":"Bob","email":"bob@example.com"}]"#.into(),
            cc_addrs: "[]".into(),
            date_sent: 1_750_000_000,
            body_text: Some("Hello\nFrom me to you\n>From quoted".into()),
            has_attachments: 0,
        }
    }

    #[test]
    fn writes_separator_headers_and_escapes_from_lines() {
        let mut out: Vec<u8> = Vec::new();
        write_mail(&mut out, &row()).unwrap();
        let text = String::from_utf8(out).unwrap();

        assert!(text.starts_with("From alice@example.com "));
        assert!(text.contains("From: Alice <alice@example.com>\r\n"));
        assert!(text.contains("To: Bob <bob@example.com>\r\n"));
        assert!(text.contains("Subject: Invoice for May\r\n"));
        assert!(text.contains("Message-ID: <m1@example.com>\r\n"));
        // Body separators escaped:
        assert!(text.contains("\r\n>From me to you\r\n"));
        assert!(text.contains("\r\n>>From quoted\r\n"));
        // Exactly one real separator line at the start of a line:
        let count = text
            .lines()
            .filter(|l| l.starts_with("From ") && !l.starts_with("From:"))
            .count();
        assert_eq!(count, 1);
    }
}
