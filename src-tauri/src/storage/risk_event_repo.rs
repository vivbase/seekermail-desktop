//! `risk_events` read + resolve persistence (T071, Module E).
//!
//! Rows are *created* elsewhere — the D1 legal analyzer ([`crate::ai::legal`])
//! and the E4 sensitive-content router ([`crate::ai::pipeline::e4_router`]) own
//! the inserts. This module owns the two operations the human-facing surface
//! needs: listing open events (T4 banner + Report risk panel) and moving one to
//! a terminal `resolved`/`dismissed` state.
//!
//! T4 invariant (root CLAUDE.md, AI_MODES_DESIGN §8.1): a level-4 event is
//! **non-dismissable** — it can only be `resolved`. [`resolve`] enforces this
//! server-side so a client bug can never silence a T4 risk.

use sqlx::Row;

use super::{map_sqlx_err, Db};
use crate::error::{AppError, AppResult};
use crate::types::{ListRiskEventsParams, ResolveRiskParams, RiskEvent};
use crate::util::now_unix;

/// The non-dismissable risk tier (T4 = `risk_level` 4).
const T4_RISK_LEVEL: i64 = 4;

/// Columns selected for a [`RiskEvent`] DTO, in struct order.
const COLS: &str = "id, mail_id, account_id, risk_level, risk_type, evidence, \
     description, status, expires_at, created_at";

fn row_to_risk_event(row: &sqlx::sqlite::SqliteRow) -> RiskEvent {
    let evidence_raw: String = row.get("evidence");
    // Evidence is stored as a JSON object string; a malformed value degrades to
    // an empty object rather than failing the whole list (the panel only reads
    // it opportunistically).
    let evidence = serde_json::from_str(&evidence_raw)
        .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));
    RiskEvent {
        id: row.get("id"),
        mail_id: row.get("mail_id"),
        account_id: row.get("account_id"),
        risk_level: row.get("risk_level"),
        risk_type: row.get("risk_type"),
        evidence,
        description: row.get("description"),
        status: row.get("status"),
        expires_at: row.get("expires_at"),
        created_at: row.get("created_at"),
    }
}

/// List risk events matching `filter`, highest risk first then newest first
/// (T071). `status` defaults to `open` when the caller omits it, so the banner
/// and report panel see only live risks.
pub async fn list(db: &Db, filter: &ListRiskEventsParams) -> AppResult<Vec<RiskEvent>> {
    use sqlx::{QueryBuilder, Sqlite};

    let mut qb = QueryBuilder::<Sqlite>::new("SELECT ");
    qb.push(COLS).push(" FROM risk_events WHERE status = ");
    qb.push_bind(filter.status.clone().unwrap_or_else(|| "open".to_string()));
    if let Some(account_id) = filter.account_id.clone() {
        qb.push(" AND account_id = ").push_bind(account_id);
    }
    if let Some(mail_id) = filter.mail_id.clone() {
        qb.push(" AND mail_id = ").push_bind(mail_id);
    }
    if let Some(risk_level) = filter.risk_level {
        qb.push(" AND risk_level = ").push_bind(risk_level);
    }
    qb.push(" ORDER BY risk_level DESC, created_at DESC");

    let rows = qb
        .build()
        .fetch_all(db.pool())
        .await
        .map_err(map_sqlx_err)?;
    Ok(rows.iter().map(row_to_risk_event).collect())
}

/// Move one risk event to a terminal state (T071). `NOT_FOUND` if the id is
/// unknown; `VALIDATION` if `status` is not `resolved`/`dismissed`; `FORBIDDEN`
/// if a `dismissed` is requested for a T4 event (which is non-dismissable).
pub async fn resolve(db: &Db, params: &ResolveRiskParams) -> AppResult<()> {
    if params.status != "resolved" && params.status != "dismissed" {
        return Err(AppError::Validation(format!(
            "risk resolution status must be 'resolved' or 'dismissed', got '{}'",
            params.status
        )));
    }

    // Read the level first so the T4 guard can run and so an unknown id is a
    // clean NOT_FOUND (the UPDATE alone could not distinguish the two).
    let row = sqlx::query("SELECT risk_level FROM risk_events WHERE id = ?")
        .bind(&params.id)
        .fetch_optional(db.pool())
        .await
        .map_err(map_sqlx_err)?
        .ok_or(AppError::NotFound)?;
    let risk_level: i64 = row.get("risk_level");

    if params.status == "dismissed" && risk_level == T4_RISK_LEVEL {
        return Err(AppError::Forbidden(
            "T4 risk events are non-dismissable; they can only be resolved".into(),
        ));
    }

    let now = now_unix();
    sqlx::query(
        "UPDATE risk_events SET status = ?, resolution_note = ?, resolved_by = 'user', \
             resolved_at = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&params.status)
    .bind(&params.resolution_note)
    .bind(now)
    .bind(now)
    .bind(&params.id)
    .execute(db.pool())
    .await
    .map_err(map_sqlx_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A migrated in-memory DB with one account and one mail, so `risk_events`
    /// foreign keys resolve.
    async fn db_with_mail() -> Db {
        let db = Db::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
                 created_at, updated_at) VALUES ('a','a@x.com','A','imap','slate','A',0,0)",
        )
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, from_email, to_addrs, date_sent, \
                 date_received, created_at, updated_at) \
             VALUES ('m1','a','<m1>','bob@x.com','[]',0,0,0,0)",
        )
        .execute(db.pool())
        .await
        .unwrap();
        db
    }

    async fn insert_event(db: &Db, id: &str, level: i64, status: &str, created_at: i64) {
        sqlx::query(
            "INSERT INTO risk_events (id, mail_id, account_id, risk_level, risk_type, evidence, \
                 description, status, expires_at, created_at, updated_at) \
             VALUES (?, 'm1', 'a', ?, 'payment_anomaly', '{\"k\":\"v\"}', 'desc', ?, NULL, ?, ?)",
        )
        .bind(id)
        .bind(level)
        .bind(status)
        .bind(created_at)
        .bind(created_at)
        .execute(db.pool())
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn list_defaults_to_open_and_orders_by_level_then_recency() {
        let db = db_with_mail().await;
        insert_event(&db, "low-old", 2, "open", 100).await;
        insert_event(&db, "high-new", 4, "open", 300).await;
        insert_event(&db, "mid-new", 3, "open", 200).await;
        insert_event(&db, "done", 4, "resolved", 400).await;

        let events = list(&db, &ListRiskEventsParams::default()).await.unwrap();
        // Resolved row excluded by the default 'open' filter.
        let ids: Vec<&str> = events.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["high-new", "mid-new", "low-old"]);
        // Evidence comes back as a parsed JSON object, not a string.
        assert_eq!(events[0].evidence["k"], serde_json::json!("v"));
    }

    #[tokio::test]
    async fn list_filters_by_mail_level_and_status() {
        let db = db_with_mail().await;
        insert_event(&db, "open-4", 4, "open", 100).await;
        insert_event(&db, "open-3", 3, "open", 200).await;
        insert_event(&db, "resolved-4", 4, "resolved", 300).await;

        let only_4 = list(
            &db,
            &ListRiskEventsParams {
                risk_level: Some(4),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(only_4.len(), 1);
        assert_eq!(only_4[0].id, "open-4");

        let resolved = list(
            &db,
            &ListRiskEventsParams {
                status: Some("resolved".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].id, "resolved-4");

        let by_mail = list(
            &db,
            &ListRiskEventsParams {
                mail_id: Some("m1".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(by_mail.len(), 2); // both open rows
    }

    #[tokio::test]
    async fn resolve_sets_terminal_state_and_audit_columns() {
        let db = db_with_mail().await;
        insert_event(&db, "r1", 3, "open", 100).await;
        resolve(
            &db,
            &ResolveRiskParams {
                id: "r1".into(),
                status: "resolved".into(),
                resolution_note: Some("handled".into()),
            },
        )
        .await
        .unwrap();

        let row = sqlx::query(
            "SELECT status, resolution_note, resolved_by, resolved_at FROM risk_events WHERE id = 'r1'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        let status: String = row.get("status");
        let note: String = row.get("resolution_note");
        let resolved_by: String = row.get("resolved_by");
        let resolved_at: i64 = row.get("resolved_at");
        assert_eq!(status, "resolved");
        assert_eq!(note, "handled");
        assert_eq!(resolved_by, "user");
        assert!(resolved_at > 0);

        // No longer visible in the default open list.
        assert!(list(&db, &ListRiskEventsParams::default())
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn resolve_rejects_t4_dismiss_but_allows_t4_resolve() {
        let db = db_with_mail().await;
        insert_event(&db, "t4", 4, "open", 100).await;

        let dismissed = resolve(
            &db,
            &ResolveRiskParams {
                id: "t4".into(),
                status: "dismissed".into(),
                resolution_note: None,
            },
        )
        .await;
        assert!(matches!(dismissed, Err(AppError::Forbidden(_))));

        // Resolving the same T4 is allowed.
        resolve(
            &db,
            &ResolveRiskParams {
                id: "t4".into(),
                status: "resolved".into(),
                resolution_note: None,
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn resolve_unknown_id_is_not_found_and_bad_status_is_validation() {
        let db = db_with_mail().await;
        let missing = resolve(
            &db,
            &ResolveRiskParams {
                id: "nope".into(),
                status: "resolved".into(),
                resolution_note: None,
            },
        )
        .await;
        assert!(matches!(missing, Err(AppError::NotFound)));

        insert_event(&db, "r1", 3, "open", 100).await;
        let bad = resolve(
            &db,
            &ResolveRiskParams {
                id: "r1".into(),
                status: "expired".into(),
                resolution_note: None,
            },
        )
        .await;
        assert!(matches!(bad, Err(AppError::Validation(_))));
    }
}
