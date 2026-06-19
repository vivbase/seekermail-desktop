//! E3 delayed-send queue (T085 §3) — DB-backed, restart-safe.
//!
//! Passing drafts get `ai_drafts.send_after = now + 30 s` while staying
//! `status = 'pending'`. A background worker scans every
//! [`SEND_QUEUE_SCAN_PERIOD_SECS`] for due rows, *claims* each one atomically
//! (clearing `send_after` so a cancel and the sender can never both win),
//! re-checks the kill switch, and delivers through the send module's direct
//! path — deliberately NOT `schedule_send`, which would add its own 10 s
//! cancel window on top of the 30 s one already elapsed.
//!
//! Because the queue is the `send_after` column itself, app restarts need no
//! recovery code: the first scan after boot picks up every due or overdue
//! row.
//!
//! **Undo semantics (T085 §3, reconciled):** the card text both sets
//! `status='discarded'` and says the draft "returns to Pending review". The
//! shipped frontend expects the latter — its Undo toast invalidates the
//! pending-drafts queries and the card must reappear. So a cancel keeps
//! `status='pending'` and only clears `send_after`; nothing is discarded.

use std::time::Duration;

use crate::ai::audit::{decision_type, AuditEntry};
use crate::error::AppResult;
use crate::state::AppState;
use crate::storage::map_sqlx_err;
use crate::types::{AiDraft, CancelSendResult, SendMailParams};
use crate::util::now_unix;

/// Undo window before an E3 draft is actually sent (F_E3 §4.3). The frontend
/// toast duration mirrors this value.
pub const E3_SEND_DELAY_SECS: i64 = 30;
/// Due-row scan cadence. Coarse on purpose: a ≤ 5 s lag on a 30 s window is
/// invisible, and the interval scan doubles as restart recovery.
pub const SEND_QUEUE_SCAN_PERIOD_SECS: u64 = 5;

/// SMTP attempts per draft before demoting to E2 (F_E3 §6).
const SEND_RETRY_MAX: u32 = 3;
// Test builds compress the retry backoff (same approach as ai/fallback.rs —
// policy identical, waits shrink so the suite runs on the real clock).
#[cfg(not(test))]
const SEND_RETRY_BASE_MS: u64 = 1_000;
#[cfg(test)]
const SEND_RETRY_BASE_MS: u64 = 2;

/// Queue a passing draft for delayed auto-send: stamp `send_after`, keep the
/// row `pending`. No in-memory state — the DB column is the queue.
pub async fn enqueue(state: &AppState, draft_id: &str, delay_secs: i64) -> AppResult<()> {
    let now = now_unix();
    sqlx::query(
        "UPDATE ai_drafts SET send_after = ?, updated_at = ? \
         WHERE id = ? AND status = 'pending'",
    )
    .bind(now + delay_secs)
    .bind(now)
    .bind(draft_id)
    .execute(state.storage.db().pool())
    .await
    .map_err(map_sqlx_err)?;
    tracing::info!(
        event = "e3_send_queued",
        draft_id = %draft_id,
        delay_secs = delay_secs,
        "draft queued for delayed auto-send"
    );
    Ok(())
}

/// Cancel a queued auto-send within its window (the `cancel_send` IPC's E3
/// branch, T085 §3). Atomic: only a row that is still `pending` with a
/// *future* `send_after` can be cancelled; a row the worker already claimed
/// (or sent) reports `cancelled = false` and the UI shows "already sent".
/// On success the draft goes back to the Pending review queue (see module
/// docs for the reconciled semantics) and `auto_send_cancelled` is audited.
pub async fn cancel_pending_auto_send(
    state: &AppState,
    draft_id: &str,
) -> AppResult<CancelSendResult> {
    let now = now_unix();
    let result = sqlx::query(
        "UPDATE ai_drafts SET send_after = NULL, updated_at = ? \
         WHERE id = ? AND status = 'pending' AND send_after IS NOT NULL AND send_after > ?",
    )
    .bind(now)
    .bind(draft_id)
    .bind(now)
    .execute(state.storage.db().pool())
    .await
    .map_err(map_sqlx_err)?;
    if result.rows_affected() == 0 {
        return Ok(CancelSendResult { cancelled: false });
    }

    let draft = crate::ai::draft::repo::get(state.storage.db(), draft_id).await?;
    state
        .audit
        .log_await(AuditEntry {
            account_id: draft.account_id.clone(),
            mail_id: Some(draft.trigger_mail_id.clone()),
            draft_id: Some(draft.id.clone()),
            decision_type: decision_type::AUTO_SEND_CANCELLED.to_string(),
            impact: "reply".into(),
            action_description: "User cancelled the queued auto-send within the undo window."
                .into(),
            result_description: "Send aborted; draft returned to the Pending review queue.".into(),
            knowledge_refs: Vec::new(),
            knowledge_summary: None,
            ai_model: Some(draft.ai_model.clone()),
            input_tokens: None,
            output_tokens: None,
            latency_ms: None,
        })
        .await?;
    // The draft re-enters the review queue → the Pending page must refresh.
    state.events.draft_ready(
        &draft.id,
        &draft.trigger_mail_id,
        &draft.trigger_mode,
        &draft.account_id,
    );
    tracing::info!(
        event = "e3_send_cancelled",
        draft_id = %draft_id,
        account_id = %draft.account_id,
        "queued auto-send cancelled; draft back in pending review"
    );
    Ok(CancelSendResult { cancelled: true })
}

/// One scan pass: claim and deliver every due draft. Returns how many rows
/// were claimed. `now` is injected for testability.
pub async fn process_due(state: &AppState, now: i64) -> AppResult<u32> {
    let due: Vec<(String,)> = sqlx::query_as(
        "SELECT id FROM ai_drafts \
         WHERE status = 'pending' AND send_after IS NOT NULL AND send_after <= ?",
    )
    .bind(now)
    .fetch_all(state.storage.db().pool())
    .await
    .map_err(map_sqlx_err)?;

    let mut claimed = 0u32;
    for (draft_id,) in due {
        if deliver_one(state, &draft_id, now).await? {
            claimed += 1;
        }
    }
    Ok(claimed)
}

/// Claim one due draft and deliver it. Returns `false` when the claim lost a
/// race (cancelled / already handled).
async fn deliver_one(state: &AppState, draft_id: &str, now: i64) -> AppResult<bool> {
    // Atomic claim: clearing send_after here means a concurrent cancel (which
    // requires a future send_after) can no longer match — exactly one side
    // wins. A crash after this point leaves the draft pending WITHOUT a
    // send_after, i.e. safely back in human review, never re-sent blind.
    let claim = sqlx::query(
        "UPDATE ai_drafts SET send_after = NULL, updated_at = ? \
         WHERE id = ? AND status = 'pending' AND send_after IS NOT NULL AND send_after <= ?",
    )
    .bind(now_unix())
    .bind(draft_id)
    .bind(now)
    .execute(state.storage.db().pool())
    .await
    .map_err(map_sqlx_err)?;
    if claim.rows_affected() == 0 {
        return Ok(false);
    }

    let draft = crate::ai::draft::repo::get(state.storage.db(), draft_id).await?;

    // Kill-switch re-check: pausing E3 after queueing must stop the send.
    if super::e3_pipeline::e3_paused(state, now_unix()).await? {
        super::e3_pipeline::record_demotion(state, &draft, "e3_paused").await?;
        return Ok(true);
    }

    // Threading headers from the trigger mail (reply semantics, RFC 2822 —
    // same derivation as approve_draft in commands/ai.rs).
    let trigger: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT message_id, \"references\" FROM mails WHERE id = ?")
            .bind(&draft.trigger_mail_id)
            .fetch_optional(state.storage.db().pool())
            .await
            .map_err(map_sqlx_err)?;
    let (in_reply_to, references) = match trigger {
        Some((message_id, refs)) => {
            let chain = match refs.filter(|r| !r.trim().is_empty()) {
                Some(r) => format!("{r} {message_id}"),
                None => message_id.clone(),
            };
            (Some(message_id), Some(chain))
        }
        None => (None, None),
    };
    let account = crate::storage::AccountRepo::new(state.storage.db())
        .get(&draft.account_id)
        .await?;
    let message_id = crate::send::make_message_id(&account.email);
    let params = SendMailParams {
        account_id: draft.account_id.clone(),
        to: vec![draft.to_addr.clone()],
        cc: draft.cc_addrs.clone(),
        bcc: Vec::new(),
        subject: draft.subject.clone(),
        body_text: draft.body_current.clone(),
        body_html: None,
        in_reply_to,
        references,
        draft_id: None,
    };

    // Direct delivery with bounded retries — NOT schedule_send (its 10 s
    // window would stack on the 30 s one that already elapsed).
    let mut last_err = None;
    for attempt in 0..SEND_RETRY_MAX {
        match crate::send::deliver(state, &params, &message_id).await {
            Ok(()) => {
                finish_sent(state, &draft, &message_id).await?;
                return Ok(true);
            }
            Err(e) => {
                tracing::warn!(
                    event = "e3_send_attempt_failed",
                    draft_id = %draft_id,
                    attempt = attempt + 1,
                    code = e.code().as_wire(),
                    "auto-send attempt failed"
                );
                last_err = Some(e);
                if attempt + 1 < SEND_RETRY_MAX {
                    tokio::time::sleep(Duration::from_millis(SEND_RETRY_BASE_MS << attempt)).await;
                }
            }
        }
    }

    // All attempts failed → demote to E2: the draft stays pending (send_after
    // already NULL from the claim) and the human takes over (F_E3 §6).
    let code = last_err.map(|e| e.code().as_wire()).unwrap_or("UNKNOWN");
    tracing::warn!(
        event = "e3_send_failed_demoted",
        draft_id = %draft_id,
        code = code,
        "auto-send failed after retries; demoted to e2 review"
    );
    super::e3_pipeline::record_demotion(state, &draft, "smtp_failed").await?;
    Ok(true)
}

/// Success bookkeeping: status flip, `auto_reply_sent` audit, `auto:sent`
/// event (the frontend shows the 30 s-style toast and refreshes the queue).
async fn finish_sent(state: &AppState, draft: &AiDraft, message_id: &str) -> AppResult<()> {
    let sent_at = now_unix();
    crate::ai::draft::repo::mark_sent(state.storage.db(), &draft.id, sent_at).await?;
    state
        .audit
        .log_await(AuditEntry {
            account_id: draft.account_id.clone(),
            mail_id: Some(draft.trigger_mail_id.clone()),
            draft_id: Some(draft.id.clone()),
            decision_type: decision_type::AUTO_REPLY_SENT.to_string(),
            impact: "reply".into(),
            action_description:
                "E3 full-auto reply passed all checks and was sent after the undo window.".into(),
            result_description: "Reply delivered via SMTP; draft marked sent.".into(),
            knowledge_refs: draft.knowledge_refs.clone(),
            knowledge_summary: None,
            ai_model: Some(draft.ai_model.clone()),
            input_tokens: None,
            output_tokens: None,
            latency_ms: None,
        })
        .await?;
    state
        .events
        .auto_sent(&draft.id, &draft.account_id, message_id);
    tracing::info!(
        event = "auto_reply_sent",
        draft_id = %draft.id,
        account_id = %draft.account_id,
        "e3 auto-reply sent"
    );
    Ok(())
}

/// Spawn the scan loop (called once from `lib.rs` at startup). The first tick
/// fires immediately, which doubles as the restart-recovery pass for rows
/// that came due while the app was closed.
pub fn start_send_queue_worker(state: AppState) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(SEND_QUEUE_SCAN_PERIOD_SECS));
        loop {
            ticker.tick().await;
            if let Err(e) = process_due(&state, now_unix()).await {
                tracing::warn!(
                    event = "e3_send_queue_scan_failed",
                    code = e.code().as_wire(),
                    "send-queue scan failed; retrying next period"
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::new_uuid;

    async fn seed_account(state: &AppState) -> String {
        // Deliberately NOT a UUID: `send::deliver` only consults the OS
        // Keychain for parseable UUID ids, so this keeps the test hermetic
        // (same approach as the send module's own tests).
        let id = format!("acct-{}", &new_uuid()[..8]);
        let now = now_unix();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, smtp_host, smtp_port, \
                 color_token, badge_label, created_at, updated_at) \
             VALUES (?, ?, 'Work', 'imap', 'smtp.example.com', 587, 'slate', 'W', ?, ?)",
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

    async fn seed_e3_draft(state: &AppState, account_id: &str, mail_id: &str) -> String {
        let id = new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO ai_drafts (id, trigger_mail_id, account_id, to_addr, cc_addrs, \
                 subject, body_original, body_current, trigger_mode, ai_model, \
                 knowledge_refs, status, created_at, updated_at) \
             VALUES (?, ?, ?, '{\"name\":\"Daniel\",\"email\":\"daniel@vendorco.example\"}', \
                 '[]', 'Re: Renewal terms', 'Reply body.', 'Reply body.', 'E3_auto', \
                 'gpt-4o', '[]', 'pending', ?, ?)",
        )
        .bind(&id)
        .bind(mail_id)
        .bind(account_id)
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        id
    }

    async fn draft_row(state: &AppState, id: &str) -> (String, Option<i64>) {
        sqlx::query_as("SELECT status, send_after FROM ai_drafts WHERE id = ?")
            .bind(id)
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn enqueue_stamps_send_after_and_keeps_pending() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state).await;
        seed_mail(&state, "m1", &account).await;
        let draft = seed_e3_draft(&state, &account, "m1").await;

        enqueue(&state, &draft, E3_SEND_DELAY_SECS).await.unwrap();
        let (status, send_after) = draft_row(&state, &draft).await;
        assert_eq!(status, "pending");
        assert!(send_after.unwrap() > now_unix());
    }

    #[tokio::test]
    async fn due_draft_is_delivered_and_audited() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state).await;
        seed_mail(&state, "m1", &account).await;
        let draft = seed_e3_draft(&state, &account, "m1").await;
        enqueue(&state, &draft, E3_SEND_DELAY_SECS).await.unwrap();

        // Not yet due → nothing happens.
        assert_eq!(process_due(&state, now_unix()).await.unwrap(), 0);
        // Past the window → delivered through the offline transport.
        let later = now_unix() + E3_SEND_DELAY_SECS + 1;
        assert_eq!(process_due(&state, later).await.unwrap(), 1);

        let (status, send_after) = draft_row(&state, &draft).await;
        assert_eq!(status, "sent");
        assert_eq!(send_after, None);
        let (audits,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM ai_decisions WHERE decision_type = 'auto_reply_sent'",
        )
        .fetch_one(state.storage.db().pool())
        .await
        .unwrap();
        assert_eq!(audits, 1);
        // The reply landed in SENT (offline transport still persists).
        let (sent,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM mails WHERE folder = 'SENT'")
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        assert_eq!(sent, 1);
    }

    #[tokio::test]
    async fn cancel_within_window_returns_draft_to_review() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state).await;
        seed_mail(&state, "m1", &account).await;
        let draft = seed_e3_draft(&state, &account, "m1").await;
        enqueue(&state, &draft, E3_SEND_DELAY_SECS).await.unwrap();

        let result = cancel_pending_auto_send(&state, &draft).await.unwrap();
        assert!(result.cancelled);
        let (status, send_after) = draft_row(&state, &draft).await;
        assert_eq!(status, "pending", "draft must return to pending review");
        assert_eq!(send_after, None);
        let (audits,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM ai_decisions WHERE decision_type = 'auto_send_cancelled'",
        )
        .fetch_one(state.storage.db().pool())
        .await
        .unwrap();
        assert_eq!(audits, 1);

        // Second cancel and a later scan are both no-ops.
        assert!(
            !cancel_pending_auto_send(&state, &draft)
                .await
                .unwrap()
                .cancelled
        );
        assert_eq!(
            process_due(&state, now_unix() + E3_SEND_DELAY_SECS + 1)
                .await
                .unwrap(),
            0
        );
        let (sent,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM mails WHERE folder = 'SENT'")
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        assert_eq!(sent, 0, "a cancelled auto-send must never deliver");
    }

    #[tokio::test]
    async fn cancel_unknown_or_unqueued_id_is_false() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state).await;
        seed_mail(&state, "m1", &account).await;
        // Pending but never queued (no send_after) → not cancellable.
        let draft = seed_e3_draft(&state, &account, "m1").await;
        assert!(
            !cancel_pending_auto_send(&state, &draft)
                .await
                .unwrap()
                .cancelled
        );
        assert!(
            !cancel_pending_auto_send(&state, "missing")
                .await
                .unwrap()
                .cancelled
        );
    }

    #[tokio::test]
    async fn paused_kill_switch_demotes_instead_of_sending() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state).await;
        seed_mail(&state, "m1", &account).await;
        let draft = seed_e3_draft(&state, &account, "m1").await;
        enqueue(&state, &draft, E3_SEND_DELAY_SECS).await.unwrap();

        crate::storage::SettingRepo::new(state.storage.db())
            .set(
                super::super::e3_pipeline::E3_PAUSED_UNTIL_KEY,
                &(now_unix() + 3_600).to_string(),
            )
            .await
            .unwrap();

        assert_eq!(
            process_due(&state, now_unix() + E3_SEND_DELAY_SECS + 1)
                .await
                .unwrap(),
            1
        );
        let (status, _) = draft_row(&state, &draft).await;
        assert_eq!(status, "pending");
        let (sent,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM mails WHERE folder = 'SENT'")
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        assert_eq!(sent, 0);
        let (downgrades,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM ai_decisions WHERE decision_type = 'downgrade_e3_to_e2'",
        )
        .fetch_one(state.storage.db().pool())
        .await
        .unwrap();
        assert_eq!(downgrades, 1);
    }
}
