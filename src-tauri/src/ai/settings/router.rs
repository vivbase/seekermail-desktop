//! `resolve_auth_route` — the unified auth-level dispatch read (T087 §3).
//!
//! Pure DB read, no side effects: every automatic trigger path (E2 pipeline
//! T082, E3 pipeline T085) calls this at its entry and branches on the
//! decision. `Disabled` (provider `none`) is a normal state — callers return
//! early without an error or a log line (T087 §6).

use crate::error::{AppError, AppResult};
use crate::storage::{map_sqlx_err, Db};
use crate::types::AiProvider;

/// Where a mail should be routed for one account (F_E1 §4, F_E2 §4.1,
/// F_E3 §4.1). Mirrors `account_ai_settings.auth_level` 1/2/3, with
/// [`AuthRouteDecision::Disabled`] overriding every level while
/// `ai_provider = 'none'`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthRouteDecision {
    /// `auth_level = 1` — E1 only; nothing is generated without a user click.
    Manual,
    /// `auth_level = 2` — E2 pre-generates drafts; a human approves the send.
    Semi,
    /// `auth_level = 3` — E3 may send autonomously within its guardrails.
    Full,
    /// `ai_provider = 'none'` — AI is off for the account regardless of level.
    Disabled,
}

/// Read `account_ai_settings.auth_level` + `ai_provider` and map them onto an
/// [`AuthRouteDecision`]. `NOT_FOUND` when the account has no settings row.
///
/// An out-of-range stored level degrades to `Manual` (the safest mode) rather
/// than erroring, so a corrupted row can never unlock automation.
pub async fn resolve_auth_route(db: &Db, account_id: &str) -> AppResult<AuthRouteDecision> {
    let row: Option<(i64, String)> = sqlx::query_as(
        "SELECT auth_level, ai_provider FROM account_ai_settings WHERE account_id = ?",
    )
    .bind(account_id)
    .fetch_optional(db.pool())
    .await
    .map_err(map_sqlx_err)?;
    let (auth_level, provider) = row.ok_or(AppError::NotFound)?;

    if AiProvider::parse(&provider) == AiProvider::None {
        return Ok(AuthRouteDecision::Disabled);
    }
    Ok(match auth_level {
        2 => AuthRouteDecision::Semi,
        3 => AuthRouteDecision::Full,
        _ => AuthRouteDecision::Manual,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ErrorCode;
    use crate::util::new_uuid;

    async fn db() -> Db {
        let db = Db::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    async fn seed_account(db: &Db, auth_level: i64, ai_provider: &str) -> String {
        let id = new_uuid();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
                 auth_level, created_at, updated_at) VALUES (?, ?, 'Work', 'imap', 'slate', 'W', ?, 0, 0)",
        )
        .bind(&id)
        .bind(format!("{id}@example.com"))
        .bind(auth_level)
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO account_ai_settings (account_id, auth_level, ai_provider, updated_at) \
             VALUES (?, ?, ?, 0)",
        )
        .bind(&id)
        .bind(auth_level)
        .bind(ai_provider)
        .execute(db.pool())
        .await
        .unwrap();
        id
    }

    #[tokio::test]
    async fn level_one_routes_manual() {
        let db = db().await;
        let account = seed_account(&db, 1, "openai").await;
        let route = resolve_auth_route(&db, &account).await.unwrap();
        assert_eq!(route, AuthRouteDecision::Manual);
    }

    #[tokio::test]
    async fn levels_two_and_three_route_semi_and_full() {
        let db = db().await;
        let semi = seed_account(&db, 2, "anthropic").await;
        let full = seed_account(&db, 3, "ollama").await;
        assert_eq!(
            resolve_auth_route(&db, &semi).await.unwrap(),
            AuthRouteDecision::Semi
        );
        assert_eq!(
            resolve_auth_route(&db, &full).await.unwrap(),
            AuthRouteDecision::Full
        );
    }

    #[tokio::test]
    async fn provider_none_is_disabled_regardless_of_level() {
        let db = db().await;
        for level in [1, 2, 3] {
            let account = seed_account(&db, level, "none").await;
            assert_eq!(
                resolve_auth_route(&db, &account).await.unwrap(),
                AuthRouteDecision::Disabled,
                "level {level} with provider none must be Disabled"
            );
        }
    }

    #[tokio::test]
    async fn out_of_range_level_degrades_to_manual() {
        let db = db().await;
        let account = seed_account(&db, 9, "openai").await;
        assert_eq!(
            resolve_auth_route(&db, &account).await.unwrap(),
            AuthRouteDecision::Manual
        );
    }

    #[tokio::test]
    async fn missing_settings_row_is_not_found() {
        let db = db().await;
        let err = resolve_auth_route(&db, "missing").await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::NotFound);
    }
}
