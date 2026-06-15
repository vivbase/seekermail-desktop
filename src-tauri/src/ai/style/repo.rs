//! Persistence for the style profile (T075 §3, 01 `account_ai_settings`).
//!
//! The profile JSON lives in `account_ai_settings.style_profile`;
//! `style_samples_count` tracks how many sent mails fed the last run. A
//! user-pinned profile (`"pinned": true`, F_E5 §4.5) is never overwritten —
//! only its sample count is refreshed.

use crate::error::{AppError, AppResult};
use crate::storage::{map_sqlx_err, Db};
use crate::util::now_unix;

use super::profiler::StyleProfileJson;

/// Read the stored profile. `Err(NotFound)` when the account has no
/// `account_ai_settings` row at all; `Ok(None)` when the row exists but no
/// profile has been learned yet (or the stored JSON no longer parses — the
/// next save overwrites it).
pub async fn load_style_profile(db: &Db, account_id: &str) -> AppResult<Option<serde_json::Value>> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT style_profile FROM account_ai_settings WHERE account_id = ?")
            .bind(account_id)
            .fetch_optional(db.pool())
            .await
            .map_err(map_sqlx_err)?;
    let (raw,) = row.ok_or(AppError::NotFound)?;
    Ok(raw.and_then(|r| serde_json::from_str(&r).ok()))
}

/// Persist a freshly built profile and its sample count, bumping `updated_at`.
///
/// Pinned guard (F_E5 §4.5): when the stored profile carries `"pinned": true`
/// (user-edited), the profile text is left untouched and only
/// `style_samples_count` is refreshed.
pub async fn save_style_profile(
    db: &Db,
    account_id: &str,
    profile: &StyleProfileJson,
    sample_count: i64,
) -> AppResult<()> {
    if let Some(existing) = load_style_profile(db, account_id).await? {
        if existing
            .get("pinned")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            tracing::info!(
                event = "style_profile_pinned_skip",
                account_id = account_id,
                sample_count = sample_count,
                "stored profile is user-pinned; refreshing sample count only"
            );
            return save_sample_count(db, account_id, sample_count).await;
        }
    }

    let json = serde_json::to_string(profile)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize style profile: {e}")))?;
    let result = sqlx::query(
        "UPDATE account_ai_settings SET style_profile = ?, style_samples_count = ?, \
         updated_at = ? WHERE account_id = ?",
    )
    .bind(&json)
    .bind(sample_count)
    .bind(now_unix())
    .bind(account_id)
    .execute(db.pool())
    .await
    .map_err(map_sqlx_err)?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    tracing::info!(
        event = "style_profile_saved",
        account_id = account_id,
        sample_count = sample_count,
        "style profile persisted"
    );
    Ok(())
}

/// Refresh only `style_samples_count` (the pinned-profile path).
pub async fn save_sample_count(db: &Db, account_id: &str, sample_count: i64) -> AppResult<()> {
    let result = sqlx::query(
        "UPDATE account_ai_settings SET style_samples_count = ?, updated_at = ? \
         WHERE account_id = ?",
    )
    .bind(sample_count)
    .bind(now_unix())
    .bind(account_id)
    .execute(db.pool())
    .await
    .map_err(map_sqlx_err)?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::style::profiler::StyleSummary;
    use crate::ai::style::STYLE_PROFILE_VERSION;
    use crate::types::ErrorCode;
    use crate::util::new_uuid;

    async fn db() -> Db {
        let db = Db::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    async fn seed_account(db: &Db) -> String {
        let id = new_uuid();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, created_at, updated_at) \
             VALUES (?, ?, 'Work', 'imap', 'slate', 'W', 0, 0)",
        )
        .bind(&id)
        .bind(format!("{id}@example.com"))
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO account_ai_settings (account_id, auth_level, updated_at) VALUES (?, 1, 100)",
        )
        .bind(&id)
        .execute(db.pool())
        .await
        .unwrap();
        id
    }

    fn fixture_profile(account_id: &str) -> StyleProfileJson {
        StyleProfileJson {
            version: STYLE_PROFILE_VERSION,
            account_id: account_id.to_string(),
            generated_at: 1_750_000_000,
            summary: StyleSummary {
                overall_tone: "Concise and courteous; leads with the decision.".into(),
                opening_patterns: vec![
                    "Hi {name},".into(),
                    "Thanks for the update".into(),
                    "Following up on".into(),
                ],
                closing_patterns: vec![
                    "Best regards,".into(),
                    "Happy to discuss further.".into(),
                    "Thanks again,".into(),
                ],
                sentence_length: "12-18 words on average".into(),
                vocabulary: "Plain business English; contract and finance terms".into(),
                format_habit: "Short paragraphs; numbered lists for action items.".into(),
            },
            sample_snippets: vec![
                "Hi Daniel, Thanks for sending over the revised statement of work.".into(),
            ],
            pinned: false,
        }
    }

    #[tokio::test]
    async fn save_then_load_roundtrips_and_updates_count() {
        let db = db().await;
        let account = seed_account(&db).await;
        let profile = fixture_profile(&account);

        save_style_profile(&db, &account, &profile, 42)
            .await
            .unwrap();

        let loaded = load_style_profile(&db, &account).await.unwrap().unwrap();
        assert_eq!(loaded, serde_json::to_value(&profile).unwrap());

        let (count, updated_at): (i64, i64) = sqlx::query_as(
            "SELECT style_samples_count, updated_at FROM account_ai_settings WHERE account_id = ?",
        )
        .bind(&account)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(count, 42);
        assert!(updated_at > 100, "updated_at must be bumped");
    }

    #[tokio::test]
    async fn pinned_profile_is_never_overwritten_but_count_updates() {
        let db = db().await;
        let account = seed_account(&db).await;
        let pinned_json = r#"{"version":1,"generated_at":1700000000,"pinned":true,"summary":{"overall_tone":"User-edited tone"}}"#;
        sqlx::query("UPDATE account_ai_settings SET style_profile = ?, style_samples_count = 10 WHERE account_id = ?")
            .bind(pinned_json)
            .bind(&account)
            .execute(db.pool())
            .await
            .unwrap();

        save_style_profile(&db, &account, &fixture_profile(&account), 55)
            .await
            .unwrap();

        let (profile, count): (Option<String>, i64) = sqlx::query_as(
            "SELECT style_profile, style_samples_count FROM account_ai_settings WHERE account_id = ?",
        )
        .bind(&account)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(
            profile.as_deref(),
            Some(pinned_json),
            "pinned content must not change"
        );
        assert_eq!(count, 55, "sample count still refreshes");
    }

    #[tokio::test]
    async fn missing_settings_row_is_not_found() {
        let db = db().await;
        let err = load_style_profile(&db, "missing-account")
            .await
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::NotFound);

        let err = save_style_profile(
            &db,
            "missing-account",
            &fixture_profile("missing-account"),
            1,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn unreadable_stored_profile_loads_as_none_and_is_overwritable() {
        let db = db().await;
        let account = seed_account(&db).await;
        sqlx::query(
            "UPDATE account_ai_settings SET style_profile = 'not json' WHERE account_id = ?",
        )
        .bind(&account)
        .execute(db.pool())
        .await
        .unwrap();

        assert!(load_style_profile(&db, &account).await.unwrap().is_none());

        // A fresh save replaces the corrupt record.
        save_style_profile(&db, &account, &fixture_profile(&account), 21)
            .await
            .unwrap();
        assert!(load_style_profile(&db, &account).await.unwrap().is_some());
    }
}
