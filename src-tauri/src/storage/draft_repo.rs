//! Compose-draft persistence (T045). Backs `save_draft` / `get_draft` /
//! `delete_draft` over the `compose_drafts` table (migration 005).

use sqlx::Row;

use super::{map_sqlx_err, Db};
use crate::error::AppResult;
use crate::types::{Draft, Recipient, SaveDraftParams};
use crate::util::{new_uuid, now_unix};

/// Upsert a draft. `params.id == None` creates a new row; otherwise the existing
/// row is updated in place (autosave overwrites the same id).
pub async fn save(db: &Db, params: &SaveDraftParams) -> AppResult<Draft> {
    let now = now_unix();
    let to_json = recipients_json(&params.to);
    let cc_json = recipients_json(&params.cc);

    let id = match &params.id {
        Some(existing) => {
            sqlx::query(
                "UPDATE compose_drafts SET to_addrs = ?, cc_addrs = ?, subject = ?, \
                     body_text = ?, body_html = ?, in_reply_to = ?, updated_at = ? WHERE id = ?",
            )
            .bind(&to_json)
            .bind(&cc_json)
            .bind(&params.subject)
            .bind(&params.body_text)
            .bind(&params.body_html)
            .bind(&params.in_reply_to)
            .bind(now)
            .bind(existing)
            .execute(db.pool())
            .await
            .map_err(map_sqlx_err)?;
            existing.clone()
        }
        None => {
            let id = new_uuid();
            sqlx::query(
                "INSERT INTO compose_drafts (id, account_id, to_addrs, cc_addrs, subject, \
                     body_text, body_html, in_reply_to, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(&params.account_id)
            .bind(&to_json)
            .bind(&cc_json)
            .bind(&params.subject)
            .bind(&params.body_text)
            .bind(&params.body_html)
            .bind(&params.in_reply_to)
            .bind(now)
            .bind(now)
            .execute(db.pool())
            .await
            .map_err(map_sqlx_err)?;
            id
        }
    };

    Ok(Draft {
        id,
        account_id: params.account_id.clone(),
        to: params.to.clone(),
        cc: params.cc.clone(),
        subject: params.subject.clone(),
        body_text: params.body_text.clone(),
        body_html: params.body_html.clone(),
        in_reply_to: params.in_reply_to.clone(),
        updated_at: now,
    })
}

/// Fetch one draft. `NotFound` if it doesn't exist.
pub async fn get(db: &Db, id: &str) -> AppResult<Draft> {
    let row = sqlx::query(
        "SELECT id, account_id, to_addrs, cc_addrs, subject, body_text, body_html, \
             in_reply_to, updated_at FROM compose_drafts WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(db.pool())
    .await
    .map_err(map_sqlx_err)?
    .ok_or(crate::error::AppError::NotFound)?;

    Ok(Draft {
        id: row.get("id"),
        account_id: row.get("account_id"),
        to: parse_recipients(&row.get::<String, _>("to_addrs")),
        cc: parse_recipients(&row.get::<String, _>("cc_addrs")),
        subject: row.get("subject"),
        body_text: row.get("body_text"),
        body_html: row.get("body_html"),
        in_reply_to: row.get("in_reply_to"),
        updated_at: row.get("updated_at"),
    })
}

/// Delete a draft by id. Deleting a missing draft is a no-op success.
pub async fn delete(db: &Db, id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM compose_drafts WHERE id = ?")
        .bind(id)
        .execute(db.pool())
        .await
        .map_err(map_sqlx_err)?;
    Ok(())
}

fn recipients_json(rs: &[Recipient]) -> String {
    let arr: Vec<serde_json::Value> = rs
        .iter()
        .map(|r| serde_json::json!({ "name": r.name, "email": r.email }))
        .collect();
    serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into())
}

fn parse_recipients(json: &str) -> Vec<Recipient> {
    serde_json::from_str::<Vec<serde_json::Value>>(json)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| {
            let email = v.get("email")?.as_str()?.to_string();
            let name = v.get("name").and_then(|n| n.as_str()).map(String::from);
            Some(Recipient { name, email })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::account_repo::{AccountRepo, NewAccount};

    async fn db_with_account() -> (Db, String) {
        let db = Db::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        let id = new_uuid();
        AccountRepo::new(&db)
            .create(&NewAccount {
                id: id.clone(),
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
        (db, id)
    }

    fn params(account_id: &str, id: Option<String>, subject: &str) -> SaveDraftParams {
        SaveDraftParams {
            id,
            account_id: account_id.into(),
            to: vec![Recipient {
                name: None,
                email: "bob@x.com".into(),
            }],
            cc: vec![],
            subject: subject.into(),
            body_text: "draft body".into(),
            body_html: None,
            in_reply_to: None,
        }
    }

    #[tokio::test]
    async fn create_update_get_delete() {
        let (db, acc) = db_with_account().await;
        let d = save(&db, &params(&acc, None, "First")).await.unwrap();
        // Update the same id (autosave).
        let d2 = save(&db, &params(&acc, Some(d.id.clone()), "Second"))
            .await
            .unwrap();
        assert_eq!(d.id, d2.id);
        let got = get(&db, &d.id).await.unwrap();
        assert_eq!(got.subject, "Second");
        assert_eq!(got.to.len(), 1);
        delete(&db, &d.id).await.unwrap();
        assert!(matches!(
            get(&db, &d.id).await.unwrap_err(),
            crate::error::AppError::NotFound
        ));
    }
}
