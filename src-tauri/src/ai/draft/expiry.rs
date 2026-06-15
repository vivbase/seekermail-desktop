//! Draft expiry sweep (T080 §3, F_E6 §4.5).
//!
//! Pending drafts carry an `expires_at` stamp computed at insert from
//! `app_settings['ai.draft_expiry_hours']` (default 72 h; `0` stores `NULL`
//! = never expires). This background task runs once at startup — covering
//! drafts that lapsed while the app was closed — then every
//! [`EXPIRY_SWEEP_PERIOD_SECS`], marking lapsed pending drafts `expired` and
//! emitting one `draft:discarded { reason: "expired" }` per draft so the
//! Pending queue drops the card. Errors are `warn`-only; the next tick
//! retries.

use std::time::Duration;

use crate::error::AppResult;
use crate::state::AppState;
use crate::util::now_unix;

use super::repo;

/// Sweep cadence: every 30 minutes (first run at startup).
pub const EXPIRY_SWEEP_PERIOD_SECS: u64 = 1_800;

/// One sweep pass: expire every lapsed pending draft and notify the UI.
/// Returns the number of drafts expired.
pub async fn sweep_expired(state: &AppState) -> AppResult<u64> {
    let db = state.storage.db();
    let ids = repo::list_expired(db, now_unix()).await?;
    if ids.is_empty() {
        return Ok(0);
    }
    repo::mark_expired(db, &ids).await?;
    for id in &ids {
        state.events.draft_discarded(id, Some("expired"));
    }
    // Identifiers and counts only (09 §5).
    tracing::info!(
        event = "drafts_expired",
        count = ids.len(),
        "expiry sweep marked lapsed pending drafts as expired"
    );
    Ok(ids.len() as u64)
}

/// Spawn the 30-minute expiry loop (called once from `lib.rs` at startup).
/// The first tick fires immediately.
pub fn start_expiry_worker(state: AppState) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(EXPIRY_SWEEP_PERIOD_SECS));
        loop {
            ticker.tick().await; // first tick fires immediately
            if let Err(e) = sweep_expired(&state).await {
                tracing::warn!(
                    event = "draft_expiry_sweep_failed",
                    code = e.code().as_wire(),
                    "draft expiry sweep failed; retrying next period"
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
        status: &str,
        expires_at: Option<i64>,
    ) -> String {
        let id = new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO ai_drafts (id, trigger_mail_id, account_id, to_addr, cc_addrs, \
                 subject, body_original, body_current, trigger_mode, ai_model, \
                 knowledge_refs, status, expires_at, created_at, updated_at) \
             VALUES (?, 'm1', ?, '{\"name\":\"\",\"email\":\"a@b.c\"}', '[]', \
                 'Re: Renewal terms', 'Body.', 'Body.', 'E2_semi', 'gpt-4o', '[]', ?, ?, ?, ?)",
        )
        .bind(&id)
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

    #[tokio::test]
    async fn sweep_expires_lapsed_pending_drafts_only() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state).await;
        seed_mail(&state, "m1", &account).await;
        let now = now_unix();
        let lapsed_a = seed_draft(&state, &account, "pending", Some(now - 60)).await;
        let lapsed_b = seed_draft(&state, &account, "pending", Some(now - 30)).await;
        let fresh = seed_draft(&state, &account, "pending", Some(now + 3_600)).await;
        let never = seed_draft(&state, &account, "pending", None).await;

        let count = sweep_expired(&state).await.unwrap();
        assert_eq!(count, 2);
        for id in [&lapsed_a, &lapsed_b] {
            let d = repo::get(state.storage.db(), id).await.unwrap();
            assert_eq!(d.status, "expired");
            assert_eq!(d.discard_reason.as_deref(), Some("expired"));
            assert!(d.discarded_at.is_some());
        }
        assert_eq!(
            repo::get(state.storage.db(), &fresh).await.unwrap().status,
            "pending"
        );
        assert_eq!(
            repo::get(state.storage.db(), &never).await.unwrap().status,
            "pending"
        );

        // Second sweep finds nothing — the first pass consumed the backlog.
        assert_eq!(sweep_expired(&state).await.unwrap(), 0);
    }
}
