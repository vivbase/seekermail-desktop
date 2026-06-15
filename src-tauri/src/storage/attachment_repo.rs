//! `AttachmentRepo` — attachment metadata + download state (T023/T025/T026).
//!
//! Rows hold metadata only; the bytes live on disk (`DiskBlobStore`) keyed by the
//! `local_path` written here once a download completes.

use super::{map_sqlx_err, Db};
use crate::error::{AppError, AppResult};
use crate::types::{Attachment, ParsedAttachment};
use crate::util::{new_uuid, now_unix};

#[derive(sqlx::FromRow)]
struct AttachmentRow {
    id: String,
    mail_id: String,
    filename: String,
    content_type: String,
    size_bytes: i64,
    downloaded: i64,
    local_path: Option<String>,
    is_inline: i64,
    content_id: Option<String>,
    checksum_sha256: Option<String>,
}

impl From<AttachmentRow> for Attachment {
    fn from(r: AttachmentRow) -> Self {
        Attachment {
            id: r.id,
            mail_id: r.mail_id,
            filename: r.filename,
            content_type: r.content_type,
            size_bytes: r.size_bytes.max(0) as u64,
            downloaded: r.downloaded != 0,
            local_path: r.local_path,
            is_inline: r.is_inline != 0,
            content_id: r.content_id,
            checksum_sha256: r.checksum_sha256,
        }
    }
}

const COLS: &str = "id, mail_id, filename, content_type, size_bytes, downloaded, local_path, \
     is_inline, content_id, checksum_sha256";

#[derive(Clone)]
pub struct AttachmentRepo<'a> {
    db: &'a Db,
}

impl<'a> AttachmentRepo<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db }
    }

    pub async fn get(&self, id: &str) -> AppResult<Attachment> {
        let sql = format!("SELECT {COLS} FROM attachments WHERE id = ?");
        let row: Option<AttachmentRow> = sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        row.map(Attachment::from).ok_or(AppError::NotFound)
    }

    pub async fn list_by_mail(&self, mail_id: &str) -> AppResult<Vec<Attachment>> {
        let sql = format!("SELECT {COLS} FROM attachments WHERE mail_id = ? ORDER BY filename");
        let rows: Vec<AttachmentRow> = sqlx::query_as(&sql)
            .bind(mail_id)
            .fetch_all(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(rows.into_iter().map(Attachment::from).collect())
    }

    /// Insert metadata for a mail's attachments (standalone form of the inline
    /// insert in `MailRepo::upsert_batch`). Returns new attachment ids.
    pub async fn insert_metadata_batch(
        &self,
        mail_id: &str,
        attachments: &[ParsedAttachment],
    ) -> AppResult<Vec<String>> {
        let now = now_unix();
        let mut ids = Vec::with_capacity(attachments.len());
        let mut tx = self.db.pool().begin().await.map_err(map_sqlx_err)?;
        for att in attachments {
            let id = new_uuid();
            sqlx::query(
                "INSERT INTO attachments (id, mail_id, content_id, filename, content_type, \
                     size_bytes, is_inline, downloaded, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, 0, ?)",
            )
            .bind(&id)
            .bind(mail_id)
            .bind(&att.content_id)
            .bind(&att.filename)
            .bind(&att.content_type)
            .bind(att.size_bytes as i64)
            .bind(att.is_inline as i64)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_err)?;
            ids.push(id);
        }
        tx.commit().await.map_err(map_sqlx_err)?;
        Ok(ids)
    }

    /// Mark an attachment downloaded with its on-disk path + checksum.
    pub async fn set_downloaded(
        &self,
        id: &str,
        local_path: &str,
        sha256: &str,
        at: i64,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE attachments SET downloaded = 1, local_path = ?, checksum_sha256 = ?, \
                 downloaded_at = ? WHERE id = ?",
        )
        .bind(local_path)
        .bind(sha256)
        .bind(at)
        .bind(id)
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    /// Find an already-downloaded blob with this checksum (hard-link dedup).
    pub async fn find_by_sha256(&self, sha256: &str) -> AppResult<Option<String>> {
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT local_path FROM attachments WHERE checksum_sha256 = ? AND downloaded = 1 \
                 AND local_path IS NOT NULL LIMIT 1",
        )
        .bind(sha256)
        .fetch_optional(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(row.and_then(|(p,)| p))
    }

    /// Flag an attachment the server no longer has, so auto-download stops trying.
    pub async fn mark_not_available(&self, id: &str) -> AppResult<()> {
        sqlx::query("UPDATE attachments SET available = 0 WHERE id = ?")
            .bind(id)
            .execute(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(())
    }

    /// Pending document attachments for an account within its knowledge window,
    /// for the auto-download queue (T025).
    pub async fn pending_auto_for_account(
        &self,
        account_id: &str,
        boundary: i64,
    ) -> AppResult<Vec<Attachment>> {
        let sql = format!(
            "SELECT {COLS} FROM attachments WHERE downloaded = 0 AND available = 1 AND is_inline = 0 \
                 AND mail_id IN (SELECT id FROM mails WHERE account_id = ? AND date_sent >= ?)"
        );
        let rows: Vec<AttachmentRow> = sqlx::query_as(&sql)
            .bind(account_id)
            .bind(boundary)
            .fetch_all(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(rows.into_iter().map(Attachment::from).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::account_repo::{AccountRepo, NewAccount};
    use crate::storage::mail_repo::MailRepo;
    use crate::types::ParsedMail;

    async fn db_with_mail() -> (Db, String) {
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
            imap_uid: Some(1),
            message_id: "<m1@x>".into(),
            in_reply_to: None,
            references: None,
            subject: "Doc".into(),
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
                filename: "report.pdf".into(),
                content_type: "application/pdf".into(),
                size_bytes: 1234,
                content_id: None,
                is_inline: false,
                data: None,
            }],
        };
        let (_stats, ins) = MailRepo::new(&db).upsert_batch(&[mail]).await.unwrap();
        (db, ins[0].summary.id.clone())
    }

    #[tokio::test]
    async fn list_set_downloaded_and_dedup() {
        let (db, mail_id) = db_with_mail().await;
        let repo = AttachmentRepo::new(&db);
        let atts = repo.list_by_mail(&mail_id).await.unwrap();
        assert_eq!(atts.len(), 1);
        let id = &atts[0].id;
        assert!(!atts[0].downloaded);

        repo.set_downloaded(
            id,
            "acc/attachments/1970/01/m/report.pdf",
            "deadbeef",
            now_unix(),
        )
        .await
        .unwrap();
        assert!(repo.get(id).await.unwrap().downloaded);
        assert_eq!(
            repo.find_by_sha256("deadbeef").await.unwrap().as_deref(),
            Some("acc/attachments/1970/01/m/report.pdf")
        );
        assert!(repo.find_by_sha256("nope").await.unwrap().is_none());
    }
}
