//! Writes a successfully-sent mail into the local store (T043).
//!
//! After SMTP accepts a message we record it locally as a `SENT` row so it shows
//! up in the unified mailbox immediately (no IMAP round-trip). Thread association
//! reuses the existing thread when the message is a reply, else opens a new one.

use sqlx::Row;

use super::{map_sqlx_err, Db};
use crate::error::AppResult;
use crate::types::{Account, MailSummary, Recipient, SendMailParams};
use crate::util::{new_uuid, now_unix, truncate_chars};

/// Persist a sent mail (folder `SENT`, read, from this account) and return its
/// summary for the `mail:new` event.
pub async fn write_sent_mail(
    db: &Db,
    account: &Account,
    params: &SendMailParams,
    message_id: &str,
    date_sent: i64,
) -> AppResult<MailSummary> {
    let now = now_unix();
    let mut tx = db.pool().begin().await.map_err(map_sqlx_err)?;

    // Thread association: reply → existing thread; else a fresh thread.
    let thread_id = match &params.in_reply_to {
        Some(irt) if !irt.is_empty() => {
            let existing = sqlx::query(
                "SELECT thread_id FROM mails WHERE account_id = ? AND message_id = ? LIMIT 1",
            )
            .bind(&account.id)
            .bind(irt)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx_err)?
            .and_then(|r| r.get::<Option<String>, _>("thread_id"));
            match existing {
                Some(tid) => tid,
                None => new_thread(&mut tx, account, params, date_sent, now).await?,
            }
        }
        _ => new_thread(&mut tx, account, params, date_sent, now).await?,
    };

    let mail_id = new_uuid();
    let to_json = recipients_json(&params.to);
    let cc_json = recipients_json(&params.cc);
    let bcc_json = recipients_json(&params.bcc);
    let snippet = truncate_chars(params.body_text.trim(), 200);

    sqlx::query(
        "INSERT INTO mails (id, account_id, thread_id, message_id, in_reply_to, \"references\", \
             subject, from_name, from_email, to_addrs, cc_addrs, bcc_addrs, date_sent, date_received, \
             body_text, body_html, snippet, is_read, is_sent, folder, embedding_status, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, 1, 'SENT', 'pending', ?, ?)",
    )
    .bind(&mail_id)
    .bind(&account.id)
    .bind(&thread_id)
    .bind(message_id)
    .bind(&params.in_reply_to)
    .bind(&params.references)
    .bind(&params.subject)
    .bind(&account.display_name)
    .bind(&account.email)
    .bind(&to_json)
    .bind(&cc_json)
    .bind(&bcc_json)
    .bind(date_sent)
    .bind(date_sent)
    .bind(&params.body_text)
    .bind(&params.body_html)
    .bind(&snippet)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx_err)?;

    // Bump the thread's aggregates.
    sqlx::query(
        "UPDATE threads SET mail_count = mail_count + 1, \
             latest_date = MAX(latest_date, ?), snippet = ?, updated_at = ? WHERE id = ?",
    )
    .bind(date_sent)
    .bind(truncate_chars(params.body_text.trim(), 160))
    .bind(now)
    .bind(&thread_id)
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx_err)?;

    tx.commit().await.map_err(map_sqlx_err)?;

    Ok(MailSummary {
        id: mail_id,
        account_id: account.id.clone(),
        thread_id: Some(thread_id),
        subject: params.subject.clone(),
        from_name: Some(account.display_name.clone()),
        from_email: account.email.clone(),
        snippet: Some(snippet),
        date_sent,
        is_read: true,
        has_attachments: false,
    })
}

async fn new_thread(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    account: &Account,
    params: &SendMailParams,
    date_sent: i64,
    now: i64,
) -> AppResult<String> {
    let thread_id = new_uuid();
    let participants: Vec<String> = std::iter::once(account.email.clone())
        .chain(params.to.iter().map(|r| r.email.clone()))
        .collect();
    let participants_json = serde_json::to_string(&participants).unwrap_or_else(|_| "[]".into());
    sqlx::query(
        "INSERT INTO threads (id, account_id, subject, participants, mail_count, unread_count, \
             latest_date, snippet, created_at, updated_at) \
         VALUES (?, ?, ?, ?, 0, 0, ?, ?, ?, ?)",
    )
    .bind(&thread_id)
    .bind(&account.id)
    .bind(&params.subject)
    .bind(&participants_json)
    .bind(date_sent)
    .bind(truncate_chars(params.body_text.trim(), 160))
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(map_sqlx_err)?;
    Ok(thread_id)
}

/// `[{ "name": …|null, "email": … }]` JSON for the address columns.
fn recipients_json(rs: &[Recipient]) -> String {
    let arr: Vec<serde_json::Value> = rs
        .iter()
        .map(|r| serde_json::json!({ "name": r.name, "email": r.email }))
        .collect();
    serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::account_repo::{AccountRepo, NewAccount};

    async fn db_with_account() -> Db {
        let db = Db::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        AccountRepo::new(&db)
            .create(&NewAccount {
                id: new_uuid(),
                email: "me@x.com".into(),
                display_name: "Me".into(),
                provider: "imap".into(),
                imap_host: None,
                imap_port: 993,
                smtp_host: Some("smtp.x.com".into()),
                smtp_port: 587,
                color_token: "slate".into(),
                badge_label: "W".into(),
                role_type: "work".into(),
                role_description: None,
                auth_level: 1,
            })
            .await
            .unwrap();
        db
    }

    fn params(account_id: &str, in_reply_to: Option<&str>) -> SendMailParams {
        SendMailParams {
            account_id: account_id.into(),
            to: vec![Recipient {
                name: Some("Bob".into()),
                email: "bob@x.com".into(),
            }],
            cc: vec![],
            bcc: vec![],
            subject: "Re: Hi".into(),
            body_text: "Body here".into(),
            body_html: None,
            in_reply_to: in_reply_to.map(String::from),
            references: None,
            draft_id: None,
        }
    }

    #[tokio::test]
    async fn writes_sent_row_and_new_thread() {
        let db = db_with_account().await;
        let account = AccountRepo::new(&db).list().await.unwrap().remove(0);
        let summary = write_sent_mail(
            &db,
            &account,
            &params(&account.id, None),
            "<m1@seekermail.local>",
            1000,
        )
        .await
        .unwrap();
        assert!(summary.is_read);
        let (folder,): (String,) = sqlx::query_as("SELECT folder FROM mails WHERE id = ?")
            .bind(&summary.id)
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(folder, "SENT");
    }

    #[tokio::test]
    async fn reply_reuses_existing_thread() {
        let db = db_with_account().await;
        let account = AccountRepo::new(&db).list().await.unwrap().remove(0);
        // Seed an inbound mail with a known message-id + thread.
        let tid = new_uuid();
        sqlx::query("INSERT INTO threads (id,account_id,subject,participants,latest_date,created_at,updated_at) VALUES (?,?,?,'[]',0,0,0)")
            .bind(&tid).bind(&account.id).bind("Hi").execute(db.pool()).await.unwrap();
        sqlx::query("INSERT INTO mails (id,account_id,thread_id,message_id,from_email,to_addrs,subject,date_sent,date_received,created_at,updated_at) VALUES (?,?,?,?,'a@x.com','[]','Hi',0,0,0,0)")
            .bind(new_uuid()).bind(&account.id).bind(&tid).bind("<orig@x>").execute(db.pool()).await.unwrap();

        let summary = write_sent_mail(
            &db,
            &account,
            &params(&account.id, Some("<orig@x>")),
            "<m2@x>",
            2000,
        )
        .await
        .unwrap();
        assert_eq!(
            summary.thread_id.as_deref(),
            Some(tid.as_str()),
            "reply joins the original thread"
        );
    }
}
