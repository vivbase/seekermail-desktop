//! `ai_drafts` repository — the E6 draft-queue lifecycle authority (T080).
//!
//! State machine (F_E6 §4.2): `pending → edited → sent / discarded / expired`.
//! Every mutating function guards the allowed source states so UI races
//! (double-approve, edit-after-send) surface as `FORBIDDEN` instead of
//! corrupting a row. `body_original` is immutable after insert — no UPDATE in
//! this module ever touches it (T090 §6).
//!
//! This module owns the row ↔ DTO projection and the single INSERT/SELECT
//! statements for `ai_drafts`; the generation engine (`engine.rs`) and the
//! IPC commands both route through it.

use crate::error::{AppError, AppResult};
use crate::storage::{map_sqlx_err, Db, SettingRepo};
use crate::types::{AiDraft, Recipient};
use crate::util::now_unix;

/// Draft lifetime in hours when `app_settings['ai.draft_expiry_hours']` is
/// absent or unreadable (T077/T080 §3). A stored `0` disables expiry.
pub const DEFAULT_DRAFT_EXPIRY_HOURS: i64 = 72;
/// `app_settings` key for the configurable draft expiry window.
pub const DRAFT_EXPIRY_HOURS_KEY: &str = "ai.draft_expiry_hours";

/// Statuses from which a draft may still be edited / approved / discarded.
const ACTIVE_STATUSES: [&str; 2] = ["pending", "edited"];

/// Full `ai_drafts` row projection for the wire DTO.
#[derive(sqlx::FromRow)]
struct AiDraftRow {
    id: String,
    trigger_mail_id: String,
    account_id: String,
    to_addr: String,
    cc_addrs: String,
    subject: String,
    body_original: String,
    body_current: String,
    is_edited: i64,
    style_match_score: Option<f64>,
    trigger_mode: String,
    ai_model: String,
    knowledge_refs: String,
    status: String,
    send_after: Option<i64>,
    expires_at: Option<i64>,
    sent_at: Option<i64>,
    discarded_at: Option<i64>,
    discard_reason: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl From<AiDraftRow> for AiDraft {
    fn from(r: AiDraftRow) -> Self {
        AiDraft {
            id: r.id,
            trigger_mail_id: r.trigger_mail_id,
            account_id: r.account_id,
            to_addr: parse_recipient(&r.to_addr),
            cc_addrs: parse_recipients(&r.cc_addrs),
            subject: r.subject,
            body_original: r.body_original,
            body_current: r.body_current,
            is_edited: r.is_edited != 0,
            style_match_score: r.style_match_score,
            trigger_mode: r.trigger_mode,
            ai_model: r.ai_model,
            knowledge_refs: serde_json::from_str(&r.knowledge_refs).unwrap_or_default(),
            status: r.status,
            send_after: r.send_after,
            expires_at: r.expires_at,
            sent_at: r.sent_at,
            discarded_at: r.discarded_at,
            discard_reason: r.discard_reason,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

const DRAFT_SELECT_COLS: &str = "id, trigger_mail_id, account_id, to_addr, cc_addrs, subject, \
     body_original, body_current, is_edited, style_match_score, trigger_mode, ai_model, \
     knowledge_refs, status, send_after, expires_at, sent_at, discarded_at, discard_reason, \
     created_at, updated_at";

// ── Reads ─────────────────────────────────────────────────────────────────────

/// Read one draft as the wire DTO. `NOT_FOUND` when absent.
pub async fn get(db: &Db, id: &str) -> AppResult<AiDraft> {
    let sql = format!("SELECT {DRAFT_SELECT_COLS} FROM ai_drafts WHERE id = ?");
    let row: Option<AiDraftRow> = sqlx::query_as(&sql)
        .bind(id)
        .fetch_optional(db.pool())
        .await
        .map_err(map_sqlx_err)?;
    row.map(AiDraft::from).ok_or(AppError::NotFound)
}

/// The E6 review queue, newest first: drafts still awaiting a human verdict.
/// `edited` is included deliberately — a user edit keeps the draft in the
/// queue until approve/discard (F_E6 §4.2; the frontend contract in
/// `src/ipc/client.ts` encodes the same semantics). Sent, discarded, and
/// expired drafts never appear here.
pub async fn list_pending(
    db: &Db,
    account_id: Option<&str>,
    limit: i64,
) -> AppResult<Vec<AiDraft>> {
    let sql = format!(
        "SELECT {DRAFT_SELECT_COLS} FROM ai_drafts \
         WHERE status IN ('pending', 'edited') AND (? IS NULL OR account_id = ?) \
         ORDER BY created_at DESC LIMIT ?"
    );
    let rows: Vec<AiDraftRow> = sqlx::query_as(&sql)
        .bind(account_id)
        .bind(account_id)
        .bind(limit)
        .fetch_all(db.pool())
        .await
        .map_err(map_sqlx_err)?;
    Ok(rows.into_iter().map(AiDraft::from).collect())
}

/// Pending drafts whose `expires_at` has passed (`NULL` = never expires).
pub async fn list_expired(db: &Db, now: i64) -> AppResult<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT id FROM ai_drafts \
         WHERE status = 'pending' AND expires_at IS NOT NULL AND expires_at < ?",
    )
    .bind(now)
    .fetch_all(db.pool())
    .await
    .map_err(map_sqlx_err)?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

// ── Insert (generation paths: T077 E1, T082 E2, T085 E3) ─────────────────────

/// Insert payload for a fresh draft (status always starts at `pending`;
/// `body_original` and `body_current` start identical).
pub struct NewAiDraft<'a> {
    pub id: &'a str,
    pub trigger_mail_id: &'a str,
    pub account_id: &'a str,
    /// JSON object `{"name":"","email":""}`.
    pub to_addr_json: &'a str,
    pub subject: &'a str,
    pub body: &'a str,
    /// `E1_manual` | `E2_semi` | `E3_auto`.
    pub trigger_mode: &'a str,
    pub ai_model: &'a str,
    /// JSON array of GTE source mail ids.
    pub knowledge_refs_json: &'a str,
    /// `None` = never expires (expiry setting `0`).
    pub expires_at: Option<i64>,
    pub created_at: i64,
}

/// The ONE `ai_drafts` INSERT in the codebase. Runs inside the caller's
/// transaction so the draft and its audit record commit together.
pub async fn insert_draft_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    draft: &NewAiDraft<'_>,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO ai_drafts (id, trigger_mail_id, account_id, to_addr, cc_addrs, subject, \
             body_original, body_current, trigger_mode, ai_model, knowledge_refs, status, \
             expires_at, created_at, updated_at) \
         VALUES (?, ?, ?, ?, '[]', ?, ?, ?, ?, ?, ?, 'pending', ?, ?, ?)",
    )
    .bind(draft.id)
    .bind(draft.trigger_mail_id)
    .bind(draft.account_id)
    .bind(draft.to_addr_json)
    .bind(draft.subject)
    .bind(draft.body)
    .bind(draft.body)
    .bind(draft.trigger_mode)
    .bind(draft.ai_model)
    .bind(draft.knowledge_refs_json)
    .bind(draft.expires_at)
    .bind(draft.created_at)
    .bind(draft.created_at)
    .execute(&mut **tx)
    .await
    .map_err(map_sqlx_err)?;
    Ok(())
}

// ── State transitions ─────────────────────────────────────────────────────────

/// Apply a user edit: update `body_current`, set `is_edited`, move the status
/// to `edited`. `body_original` is never touched. Only `pending`/`edited`
/// drafts may be edited — anything else is `FORBIDDEN` (F_E6 §4.2).
pub async fn update_body(db: &Db, id: &str, body_current: &str) -> AppResult<AiDraft> {
    let current = get(db, id).await?;
    if !ACTIVE_STATUSES.contains(&current.status.as_str()) {
        return Err(AppError::Forbidden(format!(
            "a draft in status '{}' cannot be edited",
            current.status
        )));
    }
    sqlx::query(
        "UPDATE ai_drafts SET body_current = ?, is_edited = 1, status = 'edited', \
             updated_at = ? \
         WHERE id = ? AND status IN ('pending', 'edited')",
    )
    .bind(body_current)
    .bind(now_unix())
    .bind(id)
    .execute(db.pool())
    .await
    .map_err(map_sqlx_err)?;
    get(db, id).await
}

const MARK_SENT_SQL: &str =
    "UPDATE ai_drafts SET status = 'sent', sent_at = ?, updated_at = ? WHERE id = ?";

/// Mark a draft sent (non-transactional path, e.g. the future E3 pipeline).
/// Idempotent: re-marking refreshes `sent_at` without erroring. The SMTP
/// message id is returned to callers by the send service — `ai_drafts` has no
/// column for it (dev/01), so it is intentionally not persisted here.
pub async fn mark_sent(db: &Db, id: &str, sent_at: i64) -> AppResult<()> {
    sqlx::query(MARK_SENT_SQL)
        .bind(sent_at)
        .bind(now_unix())
        .bind(id)
        .execute(db.pool())
        .await
        .map_err(map_sqlx_err)?;
    Ok(())
}

/// [`mark_sent`] inside an open transaction, so `approve_draft` commits the
/// status flip and its `draft_sent` audit record atomically (T090 §6).
pub async fn mark_sent_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: &str,
    sent_at: i64,
) -> AppResult<()> {
    sqlx::query(MARK_SENT_SQL)
        .bind(sent_at)
        .bind(now_unix())
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(map_sqlx_err)?;
    Ok(())
}

/// Discard a draft (`reason`: `user` | `superseded` | …). Idempotent: an
/// already-discarded draft keeps its original `discarded_at`/reason, and a
/// sent draft is never rewritten.
pub async fn mark_discarded(db: &Db, id: &str, reason: &str) -> AppResult<()> {
    let now = now_unix();
    sqlx::query(
        "UPDATE ai_drafts SET status = 'discarded', discard_reason = ?, discarded_at = ?, \
             updated_at = ? \
         WHERE id = ? AND status NOT IN ('discarded', 'sent')",
    )
    .bind(reason)
    .bind(now)
    .bind(now)
    .bind(id)
    .execute(db.pool())
    .await
    .map_err(map_sqlx_err)?;
    Ok(())
}

/// Expire a batch of pending drafts (the 30-minute sweep, T080 §3). Only
/// rows still `pending` move; a draft approved between scan and update wins.
pub async fn mark_expired(db: &Db, ids: &[String]) -> AppResult<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let placeholders = vec!["?"; ids.len()].join(", ");
    let sql = format!(
        "UPDATE ai_drafts SET status = 'expired', discard_reason = 'expired', \
             discarded_at = ?, updated_at = ? \
         WHERE id IN ({placeholders}) AND status = 'pending'"
    );
    let now = now_unix();
    let mut query = sqlx::query(&sql).bind(now).bind(now);
    for id in ids {
        query = query.bind(id);
    }
    query.execute(db.pool()).await.map_err(map_sqlx_err)?;
    Ok(())
}

/// T090 edge-case backstop: the 5 s undo window lives on the frontend (a
/// delayed `approve_draft` invoke), so by the time this runs the draft is
/// normally still `pending`/`edited` — a no-op that returns the row. A draft
/// already `sent` can no longer be cancelled (`FORBIDDEN`; the repo has no
/// `CONFLICT` wire code — see commands/ai.rs).
pub async fn cancel_send(db: &Db, id: &str) -> AppResult<AiDraft> {
    let draft = get(db, id).await?;
    if draft.status == "sent" {
        return Err(AppError::Forbidden(
            "draft already sent; the send can no longer be cancelled".into(),
        ));
    }
    Ok(draft)
}

// ── Expiry configuration ──────────────────────────────────────────────────────

/// The configured draft expiry window in hours (`0` = drafts never expire).
/// Single source for the generation paths and the sweep (T080 §3).
pub async fn draft_expiry_hours(db: &Db) -> AppResult<i64> {
    let raw = SettingRepo::new(db).get(DRAFT_EXPIRY_HOURS_KEY).await?;
    Ok(raw
        .and_then(|v| serde_json::from_str::<i64>(&v).ok())
        .unwrap_or(DEFAULT_DRAFT_EXPIRY_HOURS)
        .max(0))
}

// ── Stored-JSON parsing ───────────────────────────────────────────────────────

/// Parse the stored `to_addr` JSON object (`{"name":"","email":""}`).
fn parse_recipient(raw: &str) -> Recipient {
    let value: serde_json::Value = serde_json::from_str(raw).unwrap_or_default();
    Recipient {
        name: value
            .get("name")
            .and_then(|n| n.as_str())
            .filter(|n| !n.is_empty())
            .map(String::from),
        email: value
            .get("email")
            .and_then(|e| e.as_str())
            .unwrap_or_default()
            .to_string(),
    }
}

/// Parse a stored recipient array (`cc_addrs`).
fn parse_recipients(raw: &str) -> Vec<Recipient> {
    serde_json::from_str::<Vec<serde_json::Value>>(raw)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| {
            let email = v.get("email")?.as_str()?.to_string();
            let name = v
                .get("name")
                .and_then(|n| n.as_str())
                .filter(|n| !n.is_empty())
                .map(String::from);
            Some(Recipient { name, email })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use crate::types::ErrorCode;
    use crate::util::new_uuid;

    async fn seed_account(state: &AppState) -> String {
        let id = new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
                 created_at, updated_at) VALUES (?, ?, 'Work', 'imap', 'slate', 'W', ?, ?)",
        )
        .bind(&id)
        .bind(format!("{id}@example.com"))
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        id
    }

    async fn seed_mail(state: &AppState, id: &str, account_id: &str) {
        let now = now_unix();
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, subject, from_email, to_addrs, \
                 date_sent, date_received, created_at, updated_at) \
             VALUES (?, ?, ?, 'Renewal terms', 'daniel@vendorco.example', '[]', ?, ?, 0, 0)",
        )
        .bind(id)
        .bind(account_id)
        .bind(format!("<{id}@x>"))
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    async fn seed_draft(
        state: &AppState,
        account_id: &str,
        mail_id: &str,
        status: &str,
        expires_at: Option<i64>,
    ) -> String {
        let id = new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO ai_drafts (id, trigger_mail_id, account_id, to_addr, cc_addrs, \
                 subject, body_original, body_current, trigger_mode, ai_model, \
                 knowledge_refs, status, expires_at, created_at, updated_at) \
             VALUES (?, ?, ?, '{\"name\":\"Daniel\",\"email\":\"daniel@vendorco.example\"}', \
                 '[]', 'Re: Renewal terms', 'Original body.', 'Original body.', 'E2_semi', \
                 'gpt-4o', '[]', ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(mail_id)
        .bind(account_id)
        .bind(status)
        .bind(expires_at)
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        id
    }

    async fn setup() -> (AppState, String, String) {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state).await;
        seed_mail(&state, "m1", &account).await;
        (state, account, "m1".into())
    }

    #[tokio::test]
    async fn list_pending_filters_status_and_account() {
        let (state, account, mail) = setup().await;
        let db = state.storage.db();
        let p1 = seed_draft(&state, &account, &mail, "pending", None).await;
        let e1 = seed_draft(&state, &account, &mail, "edited", None).await;
        seed_draft(&state, &account, &mail, "sent", None).await;
        seed_draft(&state, &account, &mail, "discarded", None).await;
        seed_draft(&state, &account, &mail, "expired", None).await;

        // The review queue holds pending AND edited drafts — never sent /
        // discarded / expired ones (F_E6 §4.2).
        let queue = list_pending(db, None, 50).await.unwrap();
        assert_eq!(queue.len(), 2);
        let ids: Vec<&str> = queue.iter().map(|d| d.id.as_str()).collect();
        assert!(ids.contains(&p1.as_str()));
        assert!(ids.contains(&e1.as_str()));

        let other = list_pending(db, Some("other-account"), 50).await.unwrap();
        assert!(other.is_empty());
        let scoped = list_pending(db, Some(&account), 50).await.unwrap();
        assert_eq!(scoped.len(), 2);
    }

    #[tokio::test]
    async fn update_body_edits_pending_and_never_touches_original() {
        let (state, account, mail) = setup().await;
        let db = state.storage.db();
        let id = seed_draft(&state, &account, &mail, "pending", None).await;

        let updated = update_body(db, &id, "Edited body.").await.unwrap();
        assert_eq!(updated.body_current, "Edited body.");
        assert_eq!(updated.body_original, "Original body.");
        assert!(updated.is_edited);
        assert_eq!(updated.status, "edited");

        // A second edit while in `edited` is allowed.
        let again = update_body(db, &id, "Edited twice.").await.unwrap();
        assert_eq!(again.body_current, "Edited twice.");
        assert_eq!(again.status, "edited");
    }

    #[tokio::test]
    async fn update_body_on_sent_or_discarded_is_forbidden() {
        let (state, account, mail) = setup().await;
        let db = state.storage.db();
        for status in ["sent", "discarded", "expired"] {
            let id = seed_draft(&state, &account, &mail, status, None).await;
            let err = update_body(db, &id, "nope").await.unwrap_err();
            assert_eq!(err.code(), ErrorCode::Forbidden, "status {status}");
        }
        let err = update_body(db, "missing", "nope").await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn mark_sent_and_mark_discarded_are_idempotent() {
        let (state, account, mail) = setup().await;
        let db = state.storage.db();
        let id = seed_draft(&state, &account, &mail, "pending", None).await;

        let t = now_unix();
        mark_sent(db, &id, t).await.unwrap();
        mark_sent(db, &id, t).await.unwrap(); // no crash, still sent
        let draft = get(db, &id).await.unwrap();
        assert_eq!(draft.status, "sent");
        assert_eq!(draft.sent_at, Some(t));

        let id2 = seed_draft(&state, &account, &mail, "pending", None).await;
        mark_discarded(db, &id2, "user").await.unwrap();
        mark_discarded(db, &id2, "superseded").await.unwrap(); // keeps first reason
        let draft2 = get(db, &id2).await.unwrap();
        assert_eq!(draft2.status, "discarded");
        assert_eq!(draft2.discard_reason.as_deref(), Some("user"));

        // A sent draft is never rewritten by discard.
        mark_discarded(db, &id, "user").await.unwrap();
        assert_eq!(get(db, &id).await.unwrap().status, "sent");
    }

    #[tokio::test]
    async fn expiry_scan_skips_null_and_future_expirations() {
        let (state, account, mail) = setup().await;
        let db = state.storage.db();
        let now = now_unix();
        let past1 = seed_draft(&state, &account, &mail, "pending", Some(now - 10)).await;
        let past2 = seed_draft(&state, &account, &mail, "pending", Some(now - 5)).await;
        let future = seed_draft(&state, &account, &mail, "pending", Some(now + 3_600)).await;
        let never = seed_draft(&state, &account, &mail, "pending", None).await;
        let sent = seed_draft(&state, &account, &mail, "sent", Some(now - 10)).await;

        let mut expired = list_expired(db, now).await.unwrap();
        expired.sort();
        let mut want = vec![past1.clone(), past2.clone()];
        want.sort();
        assert_eq!(expired, want);

        mark_expired(db, &expired).await.unwrap();
        for id in [&past1, &past2] {
            let d = get(db, id).await.unwrap();
            assert_eq!(d.status, "expired");
            assert_eq!(d.discard_reason.as_deref(), Some("expired"));
        }
        assert_eq!(get(db, &future).await.unwrap().status, "pending");
        assert_eq!(get(db, &never).await.unwrap().status, "pending");
        assert_eq!(get(db, &sent).await.unwrap().status, "sent");
    }

    #[tokio::test]
    async fn cancel_send_rejects_sent_and_passes_through_active() {
        let (state, account, mail) = setup().await;
        let db = state.storage.db();
        let sent = seed_draft(&state, &account, &mail, "sent", None).await;
        let err = cancel_send(db, &sent).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::Forbidden);

        let pending = seed_draft(&state, &account, &mail, "pending", None).await;
        let draft = cancel_send(db, &pending).await.unwrap();
        assert_eq!(draft.status, "pending");
        // No side effects.
        assert_eq!(get(db, &pending).await.unwrap().status, "pending");
    }

    #[tokio::test]
    async fn expiry_hours_setting_defaults_and_clamps() {
        let (state, _rx) = AppState::test_state().await;
        let db = state.storage.db();
        assert_eq!(draft_expiry_hours(db).await.unwrap(), 72);
        SettingRepo::new(db)
            .set(DRAFT_EXPIRY_HOURS_KEY, "24")
            .await
            .unwrap();
        assert_eq!(draft_expiry_hours(db).await.unwrap(), 24);
        SettingRepo::new(db)
            .set(DRAFT_EXPIRY_HOURS_KEY, "0")
            .await
            .unwrap();
        assert_eq!(draft_expiry_hours(db).await.unwrap(), 0);
        SettingRepo::new(db)
            .set(DRAFT_EXPIRY_HOURS_KEY, "-5")
            .await
            .unwrap();
        assert_eq!(draft_expiry_hours(db).await.unwrap(), 0);
    }

    #[test]
    fn recipient_parsing_handles_empty_names_and_garbage() {
        let r = parse_recipient(r#"{"name":"","email":"a@b.c"}"#);
        assert_eq!(r.name, None);
        assert_eq!(r.email, "a@b.c");
        let r = parse_recipient("not json");
        assert_eq!(r.email, "");
        let list = parse_recipients(r#"[{"name":"Ana","email":"ana@x.y"}]"#);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name.as_deref(), Some("Ana"));
    }
}
