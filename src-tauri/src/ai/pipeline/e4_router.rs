//! E4 routing execution (T084 §3): apply a classification outcome.
//!
//! * `Spam` → soft-move to Trash (`folder = 'TRASH'`, `is_deleted = 1`) +
//!   `spam_trashed` audit row. No draft, no notification.
//! * `Sensitive` → `risk_events` row (level 2, expires in 7 days) + a forced
//!   review draft through the shared generation path (`trigger_mode =
//!   'E2_semi'` — the user must review, AI_MODES §5.3) + `sensitive_intercepted`
//!   audit + `risk:alert` event. The generation path emits `draft:ready`
//!   itself; a provider failure keeps the interception (risk event + audit)
//!   and is `warn`-logged — the mail still lands in front of the human.
//! * `Normal` → proceed to the calling pipeline.

use serde_json::json;

use crate::ai::audit::{decision_type, AuditEntry};
use crate::ai::draft::prompt_builder::TriggerMode;
use crate::error::AppResult;
use crate::state::AppState;
use crate::storage::map_sqlx_err;
use crate::util::{new_uuid, now_unix};

use super::e4_classifier::E4Outcome;
use super::PipelineMail;

/// E4 risk events expire after 7 days (T084 §6 — unlike T4, which never does).
pub const E4_RISK_EXPIRY_SECS: i64 = 7 * 86_400;
/// `risk_events.risk_level` reserved for E4 interceptions (T1–T6 use 1–6 as
/// inquiry levels; level 2 marks the E4 pre-scan bucket per T084 §3).
pub const E4_RISK_LEVEL: i64 = 2;

/// Where the mail went (T084 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailRouteDecision {
    Trashed,
    SensitiveDraft,
    Proceed,
}

/// Execute one routing decision. See module docs for the side effects.
pub async fn route_mail(
    state: &AppState,
    outcome: E4Outcome,
    mail: &PipelineMail,
) -> AppResult<MailRouteDecision> {
    match outcome {
        E4Outcome::Normal => Ok(MailRouteDecision::Proceed),
        E4Outcome::Spam => {
            sqlx::query(
                "UPDATE mails SET folder = 'TRASH', is_deleted = 1, updated_at = ? WHERE id = ?",
            )
            .bind(now_unix())
            .bind(&mail.id)
            .execute(state.storage.db().pool())
            .await
            .map_err(map_sqlx_err)?;
            state
                .audit
                .log_await(AuditEntry {
                    account_id: mail.account_id.clone(),
                    mail_id: Some(mail.id.clone()),
                    draft_id: None,
                    decision_type: decision_type::SPAM_TRASHED.to_string(),
                    impact: "rule".into(),
                    action_description: "E4 pre-scan classified the mail as spam.".into(),
                    result_description: "Mail moved to Trash; no draft generated.".into(),
                    knowledge_refs: Vec::new(),
                    knowledge_summary: None,
                    ai_model: None,
                    input_tokens: None,
                    output_tokens: None,
                    latency_ms: None,
                })
                .await?;
            tracing::info!(
                event = "e4_spam_trashed",
                mail_id = %mail.id,
                account_id = %mail.account_id,
                "e4 routed mail to trash"
            );
            Ok(MailRouteDecision::Trashed)
        }
        E4Outcome::Sensitive { reason, risk_type } => {
            let risk_event_id = new_uuid();
            let now = now_unix();
            let evidence = json!({
                "source": "e4_pre_scan",
                "risk_type": risk_type,
            })
            .to_string();
            sqlx::query(
                "INSERT INTO risk_events (id, mail_id, account_id, risk_level, risk_type, \
                     evidence, description, status, expires_at, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, 'open', ?, ?, ?)",
            )
            .bind(&risk_event_id)
            .bind(&mail.id)
            .bind(&mail.account_id)
            .bind(E4_RISK_LEVEL)
            .bind(&risk_type)
            .bind(&evidence)
            .bind(&reason)
            .bind(now + E4_RISK_EXPIRY_SECS)
            .bind(now)
            .bind(now)
            .execute(state.storage.db().pool())
            .await
            .map_err(map_sqlx_err)?;

            // Forced review draft (never auto-sent — E2 semantics). A
            // generation failure must not undo the interception: the risk
            // event and audit row stand, and the mail waits for the human.
            let draft_id = match crate::ai::draft::engine::generate_and_store(
                state,
                &mail.id,
                TriggerMode::E2Semi,
                None,
            )
            .await
            {
                Ok(draft) => Some(draft.id),
                Err(e) => {
                    tracing::warn!(
                        event = "e4_forced_draft_failed",
                        mail_id = %mail.id,
                        code = e.code().as_wire(),
                        "e4 sensitive interception could not generate the review draft"
                    );
                    None
                }
            };

            state
                .audit
                .log_await(AuditEntry {
                    account_id: mail.account_id.clone(),
                    mail_id: Some(mail.id.clone()),
                    draft_id: draft_id.clone(),
                    decision_type: decision_type::SENSITIVE_INTERCEPTED.to_string(),
                    impact: "risk".into(),
                    action_description:
                        "E4 pre-scan intercepted a sensitive mail; routed to human review.".into(),
                    result_description:
                        "Risk event recorded; review draft queued in Pending (never auto-sent)."
                            .into(),
                    knowledge_refs: Vec::new(),
                    knowledge_summary: None,
                    ai_model: None,
                    input_tokens: None,
                    output_tokens: None,
                    latency_ms: None,
                })
                .await?;

            state
                .events
                .risk_alert(&risk_event_id, &mail.id, &mail.account_id);
            tracing::info!(
                event = "e4_sensitive_intercepted",
                mail_id = %mail.id,
                account_id = %mail.account_id,
                risk_type = %risk_type,
                draft_generated = draft_id.is_some(),
                "e4 routed mail to the forced-draft path"
            );
            Ok(MailRouteDecision::SensitiveDraft)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ai::mock::MockProvider;
    use crate::types::AiProvider;
    use crate::util::new_uuid;

    async fn seed_account(state: &AppState) -> String {
        let id = new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
                 role_type, role_description, created_at, updated_at) \
             VALUES (?, ?, 'Maya Chen', 'imap', 'slate', 'W', 'work', \
                 'Coordinate vendor contracts and renewals.', ?, ?)",
        )
        .bind(&id)
        .bind(format!("{id}@example.com"))
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO account_ai_settings (account_id, auth_level, ai_provider, ai_model, \
                 daily_query_limit, updated_at) VALUES (?, 2, 'openai', 'gpt-4o', 1000, ?)",
        )
        .bind(&id)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        id
    }

    async fn seed_mail(state: &AppState, id: &str, account_id: &str) -> PipelineMail {
        let now = now_unix();
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, subject, from_email, to_addrs, \
                 date_sent, date_received, body_text, created_at, updated_at) \
             VALUES (?, ?, ?, 'Renewal terms', 'daniel@vendorco.example', '[]', ?, ?, \
                 'Could you confirm the renewal terms?', 0, 0)",
        )
        .bind(id)
        .bind(account_id)
        .bind(format!("<{id}@x>"))
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        super::super::load_mail(state.storage.db(), id)
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn spam_moves_to_trash_without_a_draft() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state).await;
        let mail = seed_mail(&state, "m1", &account).await;

        let decision = route_mail(&state, E4Outcome::Spam, &mail).await.unwrap();
        assert_eq!(decision, MailRouteDecision::Trashed);

        let (folder, deleted): (String, i64) =
            sqlx::query_as("SELECT folder, is_deleted FROM mails WHERE id = 'm1'")
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        assert_eq!(folder, "TRASH");
        assert_eq!(deleted, 1);

        let (drafts,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ai_drafts")
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        assert_eq!(drafts, 0);
        let (audits,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM ai_decisions WHERE decision_type = 'spam_trashed'",
        )
        .fetch_one(state.storage.db().pool())
        .await
        .unwrap();
        assert_eq!(audits, 1);
    }

    #[tokio::test]
    async fn sensitive_writes_risk_event_and_forced_draft() {
        let (state, _rx) = AppState::test_state().await;
        state
            .ai
            .register(Arc::new(MockProvider::healthy(AiProvider::Openai)));
        let account = seed_account(&state).await;
        let mail = seed_mail(&state, "m1", &account).await;

        let decision = route_mail(
            &state,
            E4Outcome::Sensitive {
                reason: "Contains document attachment".into(),
                risk_type: "payment_anomaly".into(),
            },
            &mail,
        )
        .await
        .unwrap();
        assert_eq!(decision, MailRouteDecision::SensitiveDraft);

        let (level, risk_type, status, expires_at): (i64, String, String, Option<i64>) =
            sqlx::query_as(
                "SELECT risk_level, risk_type, status, expires_at FROM risk_events \
                 WHERE mail_id = 'm1'",
            )
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        assert_eq!(level, E4_RISK_LEVEL);
        assert_eq!(risk_type, "payment_anomaly");
        assert_eq!(status, "open");
        assert!(expires_at.unwrap() > now_unix());

        let (mode, draft_status): (String, String) = sqlx::query_as(
            "SELECT trigger_mode, status FROM ai_drafts WHERE trigger_mail_id = 'm1'",
        )
        .fetch_one(state.storage.db().pool())
        .await
        .unwrap();
        assert_eq!(mode, "E2_semi");
        assert_eq!(draft_status, "pending");

        let (audits,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM ai_decisions WHERE decision_type = 'sensitive_intercepted'",
        )
        .fetch_one(state.storage.db().pool())
        .await
        .unwrap();
        assert_eq!(audits, 1);
    }

    #[tokio::test]
    async fn normal_has_no_side_effects() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state).await;
        let mail = seed_mail(&state, "m1", &account).await;
        let decision = route_mail(&state, E4Outcome::Normal, &mail).await.unwrap();
        assert_eq!(decision, MailRouteDecision::Proceed);
        let (rows,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ai_decisions")
            .fetch_one(state.storage.db().pool())
            .await
            .unwrap();
        assert_eq!(rows, 0);
    }
}
