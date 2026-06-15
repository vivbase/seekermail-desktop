//! `SettingRepo` — the `app_settings` key/value store (03 §4).
//!
//! Values are JSON-encoded strings. T029 uses it for the remote-image sender
//! allow-list (`privacy.image_allow_senders`).

use super::{map_sqlx_err, Db};
use crate::error::AppResult;
use crate::util::now_unix;

/// Settings key holding the JSON array of senders whose remote images are allowed.
const IMAGE_ALLOW_KEY: &str = "privacy.image_allow_senders";

#[derive(Clone)]
pub struct SettingRepo<'a> {
    db: &'a Db,
}

impl<'a> SettingRepo<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Raw JSON value for a key, if present.
    pub async fn get(&self, key: &str) -> AppResult<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM app_settings WHERE key = ?")
            .bind(key)
            .fetch_optional(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(row.map(|(v,)| v))
    }

    /// Upsert a JSON value for a key.
    pub async fn set(&self, key: &str, json_value: &str) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(json_value)
        .bind(now_unix())
        .execute(self.db.pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    /// Remove a key (no-op when absent). Used for reindex checkpoints (T053).
    pub async fn delete(&self, key: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM app_settings WHERE key = ?")
            .bind(key)
            .execute(self.db.pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(())
    }

    /// The remote-image sender allow-list (normalised lowercase emails).
    pub async fn get_image_allow_senders(&self) -> AppResult<Vec<String>> {
        let raw = self.get(IMAGE_ALLOW_KEY).await?;
        Ok(raw
            .and_then(|v| serde_json::from_str::<Vec<String>>(&v).ok())
            .unwrap_or_default())
    }

    /// Append a sender to the allow-list (idempotent — duplicates are ignored).
    pub async fn add_image_allow_sender(&self, sender_email: &str) -> AppResult<()> {
        let email = sender_email.trim().to_lowercase();
        let mut list = self.get_image_allow_senders().await?;
        if !list.iter().any(|e| e == &email) {
            list.push(email);
            let json = serde_json::to_string(&list).unwrap_or_else(|_| "[]".into());
            self.set(IMAGE_ALLOW_KEY, &json).await?;
        }
        Ok(())
    }

    /// Is this sender already allowed to load remote images?
    pub async fn is_sender_image_allowed(&self, sender_email: &str) -> AppResult<bool> {
        let email = sender_email.trim().to_lowercase();
        Ok(self.get_image_allow_senders().await?.contains(&email))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> Db {
        let db = Db::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    #[tokio::test]
    async fn allow_list_is_idempotent_and_case_insensitive() {
        let db = db().await;
        let repo = SettingRepo::new(&db);
        repo.add_image_allow_sender("News@Example.com")
            .await
            .unwrap();
        repo.add_image_allow_sender("news@example.com")
            .await
            .unwrap();
        let list = repo.get_image_allow_senders().await.unwrap();
        assert_eq!(list, vec!["news@example.com".to_string()]);
        assert!(repo
            .is_sender_image_allowed("NEWS@example.com")
            .await
            .unwrap());
        assert!(!repo
            .is_sender_image_allowed("other@example.com")
            .await
            .unwrap());
    }
}
