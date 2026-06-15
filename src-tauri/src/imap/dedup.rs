//! IMAP UID de-duplication helper (T022 §3, F_A4 §4.6).
//!
//! Identity is `(account_id, folder, imap_uid)`. Fetch tasks use this to skip
//! UIDs already on disk; the DB-level unique index on `(account_id, message_id)`
//! is the final guard at upsert time.

use crate::error::AppResult;
use crate::storage::{map_sqlx_err, Db};

/// True if a mail with this `(account, folder, uid)` already exists locally.
pub async fn is_duplicate(
    db: &Db,
    account_id: &str,
    folder: &str,
    imap_uid: i64,
) -> AppResult<bool> {
    let row =
        sqlx::query("SELECT 1 FROM mails WHERE account_id = ? AND folder = ? AND imap_uid = ?")
            .bind(account_id)
            .bind(folder)
            .bind(imap_uid)
            .fetch_optional(db.pool())
            .await
            .map_err(map_sqlx_err)?;
    Ok(row.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::account_repo::{AccountRepo, NewAccount};
    use crate::storage::mail_repo::MailRepo;
    use crate::types::ParsedMail;

    #[tokio::test]
    async fn detects_existing_uid() {
        let db = Db::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        AccountRepo::new(&db)
            .create(&NewAccount {
                id: "acc".into(),
                email: "me@x.com".into(),
                display_name: "Me".into(),
                provider: "imap".into(),
                imap_host: None,
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
            imap_uid: Some(77),
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
            date_sent: 1,
            date_received: 1,
            body_text: Some("b".into()),
            body_html: None,
            snippet: Some("b".into()),
            has_attachments: false,
            tracker_count: 0,
            attachments: vec![],
        };
        MailRepo::new(&db).upsert_batch(&[mail]).await.unwrap();
        assert!(is_duplicate(&db, "acc", "INBOX", 77).await.unwrap());
        assert!(!is_duplicate(&db, "acc", "INBOX", 78).await.unwrap());
    }
}
