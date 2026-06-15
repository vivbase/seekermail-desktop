//! Agent-IM (TEAM) channel persistence (T092, F_I2 §5). Backs the
//! `post_im_message` / `list_im_messages` / `mark_im_message_read` commands over
//! the `im_messages` table (migration 008).
//!
//! The TEAM channel is a single shared room — there are no private channels.
//! `channel_id` is validated to `"main"` here and pinned by a CHECK constraint in
//! the schema, so the "no private chats" invariant holds at both layers.

use sqlx::Row;

use super::{map_sqlx_err, Db};
use crate::error::{AppError, AppResult};
use crate::types::{ImMessage, PageResult};
use crate::util::{new_uuid, now_unix};

/// The one valid channel id (no private chats — root CLAUDE.md "Agent-IM").
pub const MAIN_CHANNEL: &str = "main";

/// Retention: keep at most this many messages (oldest pruned first).
const MAX_MESSAGES: i64 = 5000;
/// Retention: drop messages older than this many seconds (90 days).
const MAX_AGE_SECS: i64 = 90 * 86_400;

/// Default / max page sizes for `list_messages` (02 §Pagination).
const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

const IM_COLS: &str = "id, channel_id, sender_type, sender_id, message_type, content, \
     linked_email_id, status, created_at, read_at";

fn row_to_message(row: &sqlx::sqlite::SqliteRow) -> ImMessage {
    ImMessage {
        id: row.get("id"),
        channel_id: row.get("channel_id"),
        sender_type: row.get("sender_type"),
        sender_id: row.get("sender_id"),
        message_type: row.get("message_type"),
        content: row.get("content"),
        linked_email_id: row.get("linked_email_id"),
        status: row.get("status"),
        created_at: row.get("created_at"),
        read_at: row.get("read_at"),
    }
}

fn validate_enum(field: &str, value: &str, allowed: &[&str]) -> AppResult<()> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "{field} must be one of {allowed:?}, got '{value}'"
        )))
    }
}

/// Insert one channel message and return it (T092). `channel_id` must be `"main"`
/// (the no-private-chats invariant); `status` defaults to `"resolved"` when `None`
/// — query cards (I3/I4) pass `Some("pending")`.
#[allow(clippy::too_many_arguments)]
pub async fn insert_message(
    db: &Db,
    channel_id: &str,
    sender_type: &str,
    sender_id: &str,
    message_type: &str,
    content: &str,
    linked_email_id: Option<&str>,
    status: Option<&str>,
) -> AppResult<ImMessage> {
    // Validate every enum-ish field for clean VALIDATION errors instead of relying
    // on raw CHECK-constraint failures (which surface as opaque DB errors).
    if channel_id != MAIN_CHANNEL {
        return Err(AppError::Validation(format!(
            "channel_id must be '{MAIN_CHANNEL}' (no private chats), got '{channel_id}'"
        )));
    }
    validate_enum("sender_type", sender_type, &["human", "agent", "system"])?;
    validate_enum(
        "message_type",
        message_type,
        &["text", "query_card", "card_reply", "status"],
    )?;
    let status = status.unwrap_or("resolved");
    validate_enum(
        "status",
        status,
        &["pending", "answered", "skipped", "resolved"],
    )?;

    let id = new_uuid();
    let now = now_unix();
    sqlx::query(
        "INSERT INTO im_messages (id, channel_id, sender_type, sender_id, message_type, \
             content, linked_email_id, status, created_at, read_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NULL)",
    )
    .bind(&id)
    .bind(channel_id)
    .bind(sender_type)
    .bind(sender_id)
    .bind(message_type)
    .bind(content)
    .bind(linked_email_id)
    .bind(status)
    .bind(now)
    .execute(db.pool())
    .await
    .map_err(map_sqlx_err)?;

    get_message(db, &id).await
}

/// Fetch one message by id (`NOT_FOUND` if absent).
pub async fn get_message(db: &Db, id: &str) -> AppResult<ImMessage> {
    let sql = format!("SELECT {IM_COLS} FROM im_messages WHERE id = ?");
    let row = sqlx::query(&sql)
        .bind(id)
        .fetch_optional(db.pool())
        .await
        .map_err(map_sqlx_err)?
        .ok_or(AppError::NotFound)?;
    Ok(row_to_message(&row))
}

/// List channel messages oldest-first with pagination + optional `sender_id` /
/// `status` filters (T092). `limit` defaults to 50 and is clamped to `[1, 200]`.
pub async fn list_messages(
    db: &Db,
    sender_id: Option<&str>,
    status: Option<&str>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> AppResult<PageResult<ImMessage>> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = offset.unwrap_or(0).max(0);

    // Build the shared WHERE clause once for both the page and the count.
    let mut filter = String::from(" WHERE channel_id = ?");
    if sender_id.is_some() {
        filter.push_str(" AND sender_id = ?");
    }
    if status.is_some() {
        filter.push_str(" AND status = ?");
    }

    let list_sql = format!(
        "SELECT {IM_COLS} FROM im_messages{filter} ORDER BY created_at ASC, id ASC \
         LIMIT ? OFFSET ?"
    );
    let mut q = sqlx::query(&list_sql).bind(MAIN_CHANNEL);
    if let Some(s) = sender_id {
        q = q.bind(s);
    }
    if let Some(s) = status {
        q = q.bind(s);
    }
    let rows = q
        .bind(limit)
        .bind(offset)
        .fetch_all(db.pool())
        .await
        .map_err(map_sqlx_err)?;
    let items: Vec<ImMessage> = rows.iter().map(row_to_message).collect();

    let count_sql = format!("SELECT count(*) FROM im_messages{filter}");
    let mut cq = sqlx::query_scalar::<_, i64>(&count_sql).bind(MAIN_CHANNEL);
    if let Some(s) = sender_id {
        cq = cq.bind(s);
    }
    if let Some(s) = status {
        cq = cq.bind(s);
    }
    let total = cq.fetch_one(db.pool()).await.map_err(map_sqlx_err)?;

    Ok(PageResult {
        items,
        total: total as u32,
        offset: offset as u32,
    })
}

/// Mark one message read (T092). Idempotent: only writes `read_at` when it is
/// still NULL, so an already-read message keeps its original timestamp.
pub async fn mark_read(db: &Db, id: &str) -> AppResult<()> {
    sqlx::query("UPDATE im_messages SET read_at = ? WHERE id = ? AND read_at IS NULL")
        .bind(now_unix())
        .bind(id)
        .execute(db.pool())
        .await
        .map_err(map_sqlx_err)?;
    Ok(())
}

/// Retention sweep (T092, F_I2 §5): drop messages older than 90 days, then prune
/// the oldest rows beyond the 5000-message cap. Returns the number deleted.
/// Called fire-and-forget after each insert, so it must never block the command.
pub async fn purge_old(db: &Db) -> AppResult<u64> {
    let cutoff = now_unix() - MAX_AGE_SECS;
    let aged = sqlx::query("DELETE FROM im_messages WHERE created_at < ?")
        .bind(cutoff)
        .execute(db.pool())
        .await
        .map_err(map_sqlx_err)?
        .rows_affected();

    let (remaining,): (i64,) = sqlx::query_as("SELECT count(*) FROM im_messages")
        .fetch_one(db.pool())
        .await
        .map_err(map_sqlx_err)?;
    let over_cap = remaining - MAX_MESSAGES;
    let capped = if over_cap > 0 {
        sqlx::query(
            "DELETE FROM im_messages WHERE id IN (\
                 SELECT id FROM im_messages ORDER BY created_at ASC, id ASC LIMIT ?\
             )",
        )
        .bind(over_cap)
        .execute(db.pool())
        .await
        .map_err(map_sqlx_err)?
        .rows_affected()
    } else {
        0
    };
    Ok(aged + capped)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> Db {
        let db = Db::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    fn text_content(s: &str) -> String {
        serde_json::json!({ "text": s }).to_string()
    }

    #[tokio::test]
    async fn insert_and_get_roundtrip() {
        let db = db().await;
        let m = insert_message(
            &db,
            "main",
            "human",
            "human",
            "text",
            &text_content("hello team"),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(m.channel_id, "main");
        assert_eq!(m.status, "resolved");
        assert!(m.read_at.is_none());
        assert_eq!(get_message(&db, &m.id).await.unwrap().id, m.id);
    }

    #[tokio::test]
    async fn non_main_channel_is_validation() {
        let db = db().await;
        let err = insert_message(
            &db,
            "workspace",
            "human",
            "human",
            "text",
            &text_content("x"),
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn bad_sender_type_is_validation() {
        let db = db().await;
        let err = insert_message(
            &db,
            "main",
            "bot",
            "x",
            "text",
            &text_content("x"),
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn list_paginates_and_filters() {
        let db = db().await;
        for i in 0..3 {
            insert_message(
                &db,
                "main",
                "agent",
                "acc-a",
                "text",
                &text_content(&format!("a{i}")),
                None,
                None,
            )
            .await
            .unwrap();
        }
        insert_message(
            &db,
            "main",
            "agent",
            "acc-b",
            "text",
            &text_content("b"),
            None,
            None,
        )
        .await
        .unwrap();

        let all = list_messages(&db, None, None, None, None).await.unwrap();
        assert_eq!(all.total, 4);
        assert_eq!(all.items.len(), 4);

        // sender filter
        let only_a = list_messages(&db, Some("acc-a"), None, None, None)
            .await
            .unwrap();
        assert_eq!(only_a.total, 3);
        assert!(only_a.items.iter().all(|m| m.sender_id == "acc-a"));

        // pagination: limit 2, offset 2
        let page = list_messages(&db, None, None, Some(2), Some(2))
            .await
            .unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.offset, 2);
        assert_eq!(page.total, 4);
    }

    #[tokio::test]
    async fn mark_read_is_idempotent() {
        let db = db().await;
        let m = insert_message(
            &db,
            "main",
            "human",
            "human",
            "text",
            &text_content("x"),
            None,
            None,
        )
        .await
        .unwrap();
        mark_read(&db, &m.id).await.unwrap();
        let first = get_message(&db, &m.id).await.unwrap().read_at;
        assert!(first.is_some());
        // A second mark must not overwrite the original timestamp.
        mark_read(&db, &m.id).await.unwrap();
        assert_eq!(get_message(&db, &m.id).await.unwrap().read_at, first);
    }

    #[tokio::test]
    async fn purge_drops_aged_and_over_cap() {
        let db = db().await;
        // One ancient message (older than 90 days) gets dropped by age.
        let old_id = new_uuid();
        sqlx::query(
            "INSERT INTO im_messages (id, channel_id, sender_type, sender_id, message_type, \
                 content, status, created_at) VALUES (?, 'main', 'human', 'human', 'text', \
                 '{\"text\":\"old\"}', 'resolved', ?)",
        )
        .bind(&old_id)
        .bind(now_unix() - 91 * 86_400)
        .execute(db.pool())
        .await
        .unwrap();
        let deleted = purge_old(&db).await.unwrap();
        assert_eq!(deleted, 1);
        assert!(matches!(
            get_message(&db, &old_id).await.unwrap_err(),
            AppError::NotFound
        ));
    }

    #[tokio::test]
    async fn purge_enforces_5000_cap() {
        let db = db().await;
        // Seed 5001 recent rows with increasing timestamps so the oldest is
        // deterministic AND none are old enough to trip the 90-day age sweep —
        // this test isolates the 5000-row cap.
        let base = now_unix();
        let mut tx = db.pool().begin().await.unwrap();
        for i in 0..5001i64 {
            sqlx::query(
                "INSERT INTO im_messages (id, channel_id, sender_type, sender_id, message_type, \
                     content, status, created_at) VALUES (?, 'main', 'agent', 'acc', 'text', \
                     '{\"text\":\"m\"}', 'resolved', ?)",
            )
            .bind(new_uuid())
            .bind(base - 5001 + i)
            .execute(&mut *tx)
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();

        purge_old(&db).await.unwrap();
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM im_messages")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(n, 5000, "oldest pruned down to the cap");
    }
}
