//! Thread resolution (T023 §3).
//!
//! Given a parsed mail, find (or create) its `thread_id` inside the ingest
//! transaction, by priority:
//! 1. `In-Reply-To` → the referenced message's thread.
//! 2. last `References` id → same.
//! 3. normalised subject + same account within 30 days → that thread.
//! 4. otherwise a new thread.

use sqlx::Row;

use crate::error::{AppError, AppResult};
use crate::types::ParsedMail;
use crate::util::{new_uuid, truncate_chars};

/// 30-day window for subject-based threading (seconds).
const SUBJECT_WINDOW_SECS: i64 = 30 * 24 * 60 * 60;

/// Strip leading `Re:` / `Fwd:` / `Fw:` prefixes (case-insensitive, repeated).
pub fn normalize_subject(subject: &str) -> String {
    let mut s = subject.trim();
    loop {
        let lower = s.to_lowercase();
        let trimmed = lower
            .strip_prefix("re:")
            .or_else(|| lower.strip_prefix("fwd:"))
            .or_else(|| lower.strip_prefix("fw:"));
        match trimmed {
            Some(_) => {
                // Cut the same number of bytes off the original (prefixes are ASCII).
                let cut = s.len() - trimmed.unwrap().len();
                s = s[cut..].trim_start();
            }
            None => break,
        }
    }
    s.to_string()
}

/// The last Message-ID in a space-separated References chain.
fn last_reference(references: &str) -> Option<&str> {
    references.split_whitespace().last()
}

async fn thread_of_message(
    conn: &mut sqlx::SqliteConnection,
    account_id: &str,
    message_id: &str,
) -> AppResult<Option<String>> {
    let row = sqlx::query("SELECT thread_id FROM mails WHERE account_id = ? AND message_id = ?")
        .bind(account_id)
        .bind(message_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(crate::storage::map_sqlx_err)?;
    Ok(row.and_then(|r| r.get::<Option<String>, _>("thread_id")))
}

/// Resolve (find or create) the thread for `mail`. Creates a thread row with zero
/// counts; [`crate::storage::mail_repo`] bumps the aggregates after inserting the
/// mail.
pub async fn resolve(
    conn: &mut sqlx::SqliteConnection,
    mail: &ParsedMail,
    now: i64,
) -> AppResult<String> {
    // 1. In-Reply-To.
    if let Some(irt) = &mail.in_reply_to {
        if let Some(tid) = thread_of_message(conn, &mail.account_id, irt).await? {
            return Ok(tid);
        }
    }
    // 2. References (last id).
    if let Some(refs) = &mail.references {
        if let Some(last) = last_reference(refs) {
            if let Some(tid) = thread_of_message(conn, &mail.account_id, last).await? {
                return Ok(tid);
            }
        }
    }
    // 3. Normalised subject within the time window.
    let norm = normalize_subject(&mail.subject);
    if !norm.is_empty() {
        let row = sqlx::query(
            "SELECT id FROM threads WHERE account_id = ? AND subject = ? AND latest_date >= ? \
             ORDER BY latest_date DESC LIMIT 1",
        )
        .bind(&mail.account_id)
        .bind(&norm)
        .bind(mail.date_sent - SUBJECT_WINDOW_SECS)
        .fetch_optional(&mut *conn)
        .await
        .map_err(crate::storage::map_sqlx_err)?;
        if let Some(r) = row {
            return Ok(r.get::<String, _>("id"));
        }
    }
    // 4. New thread.
    let id = new_uuid();
    let participants = serde_json::to_string(&[&mail.from_email]).unwrap_or_else(|_| "[]".into());
    let snippet = mail.snippet.clone().unwrap_or_default();
    sqlx::query(
        "INSERT INTO threads (id, account_id, subject, participants, mail_count, unread_count, \
             has_attachments, latest_date, snippet, created_at, updated_at) \
         VALUES (?, ?, ?, ?, 0, 0, 0, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&mail.account_id)
    .bind(&norm)
    .bind(&participants)
    .bind(mail.date_sent)
    .bind(truncate_chars(&snippet, 160))
    .bind(now)
    .bind(now)
    .execute(&mut *conn)
    .await
    .map_err(map_thread_err)?;
    Ok(id)
}

fn map_thread_err(e: sqlx::Error) -> AppError {
    crate::storage::map_sqlx_err(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_repeated_prefixes() {
        assert_eq!(normalize_subject("Re: Fwd: Hello"), "Hello");
        assert_eq!(normalize_subject("RE: re: Status"), "Status");
        assert_eq!(normalize_subject("No prefix"), "No prefix");
    }

    #[test]
    fn last_reference_picks_last() {
        assert_eq!(last_reference("<a> <b> <c>"), Some("<c>"));
        assert_eq!(last_reference(""), None);
    }
}
