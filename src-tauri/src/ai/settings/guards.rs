//! `require_auth_level` — the explicit-overreach guard (T087 §3, 09 §3).
//!
//! Background pipelines never call this: they use
//! [`super::router::resolve_auth_route`] and *skip* on a mismatch, because a
//! Manual account flowing past an automation entry point is a normal state.
//! This guard exists for the other case — a frontend (or scripted) request
//! that explicitly asks for an operation the account's level does not permit,
//! e.g. invoking the E2 pipeline command directly against a Manual-only
//! account. That request must fail loudly with `FORBIDDEN`.

use crate::error::{AppError, AppResult};
use crate::storage::{map_sqlx_err, Db};

/// Fails with `FORBIDDEN` unless the account's `auth_level` equals `required`.
/// `NOT_FOUND` when the account has no `account_ai_settings` row.
pub async fn require_auth_level(db: &Db, account_id: &str, required: u8) -> AppResult<()> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT auth_level FROM account_ai_settings WHERE account_id = ?")
            .bind(account_id)
            .fetch_optional(db.pool())
            .await
            .map_err(map_sqlx_err)?;
    let (level,) = row.ok_or(AppError::NotFound)?;
    if level != i64::from(required) {
        return Err(AppError::Forbidden(format!(
            "operation requires auth level {required}; account is at level {level}"
        )));
    }
    Ok(())
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

    async fn seed_account(db: &Db, auth_level: i64) -> String {
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
            "INSERT INTO account_ai_settings (account_id, auth_level, updated_at) VALUES (?, ?, 0)",
        )
        .bind(&id)
        .bind(auth_level)
        .execute(db.pool())
        .await
        .unwrap();
        id
    }

    #[tokio::test]
    async fn mismatch_is_forbidden() {
        let db = db().await;
        let account = seed_account(&db, 1).await;
        let err = require_auth_level(&db, &account, 2).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::Forbidden);
    }

    #[tokio::test]
    async fn exact_match_passes() {
        let db = db().await;
        let account = seed_account(&db, 2).await;
        require_auth_level(&db, &account, 2).await.unwrap();
    }

    #[tokio::test]
    async fn missing_settings_row_is_not_found() {
        let db = db().await;
        let err = require_auth_level(&db, "missing", 1).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::NotFound);
    }
}
