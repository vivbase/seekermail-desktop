//! Attachment text indexing (T109, C1+C2 deepening · v0.6).
//!
//! Consumes the `extraction_status = 'indexed'` rows T108 produces and feeds the
//! extracted text into the two search indexes:
//!
//! * **FTS5** — handled declaratively by the migration-012 trigger, which mirrors
//!   `filename` + `extracted_text` into `attachments_fts` the moment a row turns
//!   `indexed`. No code here touches FTS directly.
//! * **Vectors** — this module chunks the text (the same 400/40 chunker the mail
//!   pipeline uses, T031), embeds each chunk, and upserts one `email_vectors` row
//!   per chunk with `chunk_id = "{attachment_id}:{i}"` and the owning mail's
//!   `mail_id` / `account_id` / `date_sent`, so attachment hits aggregate into the
//!   right mail in semantic search (T033/T112).
//!
//! `embedding_att_status` tracks the vector phase: `pending` → `indexed`; rows
//! whose extraction was `skipped`/`error` are propagated to `skipped` (never
//! embedded).

use crate::embedding::chunker::chunk_mail;
use crate::embedding::{Embedder, MODEL_NAME};
use crate::error::{AppError, AppResult};
use crate::storage::{facade::StorageFacade, map_sqlx_err};
use crate::vector::VectorRow;

/// Aggregate result of one [`AttachmentIndexer::index_extracted_batch`] call.
#[derive(Debug, Clone, Copy, Default)]
pub struct IndexStats {
    /// Attachments embedded this batch.
    pub indexed: u32,
    /// Attachments still `pending` for the vector phase after this batch.
    pub remaining: u32,
}

/// One indexable attachment row (joined with its mail for date/sender).
#[derive(sqlx::FromRow)]
struct IndexRow {
    id: String,
    mail_id: String,
    account_id: String,
    filename: String,
    extracted_text: Option<String>,
    extraction_status: String,
    date_sent: i64,
}

/// Attachment → vector index service. Cheap to clone.
#[derive(Clone)]
pub struct AttachmentIndexer {
    storage: StorageFacade,
    embedder: Embedder,
}

impl AttachmentIndexer {
    pub fn new(storage: StorageFacade, embedder: Embedder) -> Self {
        Self { storage, embedder }
    }

    pub fn from_state(state: &crate::state::AppState) -> Self {
        Self::new(state.storage.clone(), state.embedder.clone())
    }

    /// Embed up to `limit` extracted-but-unembedded attachments. Propagates the
    /// `skipped`/`error` extraction states to `embedding_att_status = 'skipped'`
    /// first, then embeds the `indexed` ones.
    pub async fn index_extracted_batch(&self, limit: usize) -> AppResult<IndexStats> {
        let db = self.storage.db().pool();

        // Propagate non-indexable extraction outcomes — never embedded.
        sqlx::query(
            "UPDATE attachments SET embedding_att_status = 'skipped' \
             WHERE embedding_att_status = 'pending' AND extraction_status IN ('skipped', 'error')",
        )
        .execute(db)
        .await
        .map_err(map_sqlx_err)?;

        let rows: Vec<IndexRow> = sqlx::query_as(
            "SELECT a.id, a.mail_id, COALESCE(a.account_id, m.account_id) AS account_id, \
                    a.filename, a.extracted_text, a.extraction_status, m.date_sent \
             FROM attachments a JOIN mails m ON m.id = a.mail_id \
             WHERE a.embedding_att_status = 'pending' AND a.extraction_status = 'indexed' \
                   AND a.extracted_text IS NOT NULL \
             ORDER BY a.extracted_at LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(db)
        .await
        .map_err(map_sqlx_err)?;

        let mut indexed = 0u32;
        for row in rows {
            match self.index_row(&row).await {
                Ok(true) => indexed += 1,
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(attachment_id = %row.id, error = %e, "attachment index failed");
                    self.set_status(&row.id, "error").await.ok();
                }
            }
        }

        let (remaining,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM attachments \
             WHERE embedding_att_status = 'pending' AND extraction_status = 'indexed'",
        )
        .fetch_one(db)
        .await
        .map_err(map_sqlx_err)?;

        Ok(IndexStats {
            indexed,
            remaining: remaining.max(0) as u32,
        })
    }

    /// Index a single attachment by id (the increment path: download → extract →
    /// index). No-op (returns `Ok`) for rows that aren't extracted text.
    pub async fn index_one(&self, attachment_id: &str) -> AppResult<()> {
        let db = self.storage.db().pool();
        let row: Option<IndexRow> = sqlx::query_as(
            "SELECT a.id, a.mail_id, COALESCE(a.account_id, m.account_id) AS account_id, \
                    a.filename, a.extracted_text, a.extraction_status, m.date_sent \
             FROM attachments a JOIN mails m ON m.id = a.mail_id WHERE a.id = ?",
        )
        .bind(attachment_id)
        .fetch_optional(db)
        .await
        .map_err(map_sqlx_err)?;
        let Some(row) = row else {
            return Err(AppError::NotFound);
        };
        if row.extraction_status != "indexed" {
            self.set_status(&row.id, "skipped").await?;
            return Ok(());
        }
        self.index_row(&row).await.map(|_| ())
    }

    /// Embed one row's chunks and upsert them. Returns `Ok(true)` when vectors
    /// were written, `Ok(false)` when there was nothing to embed (skipped).
    async fn index_row(&self, row: &IndexRow) -> AppResult<bool> {
        let text = row.extracted_text.clone().unwrap_or_default();
        let chunks = chunk_mail(&text);
        if chunks.is_empty() {
            self.set_status(&row.id, "skipped").await?;
            return Ok(false);
        }

        let vectors = self.embedder.embed_batch_blocking(chunks.clone()).await?;
        let mut vrows = Vec::with_capacity(vectors.len());
        for (i, (chunk, vector)) in chunks.iter().zip(vectors).enumerate() {
            let snippet: String = chunk.chars().take(200).collect();
            vrows.push(VectorRow {
                chunk_id: format!("{}:{}", row.id, i),
                mail_id: row.mail_id.clone(),
                chunk_index: i as i32,
                account_id: row.account_id.clone(),
                from_email: String::new(),
                date_sent: row.date_sent,
                subject: row.filename.clone(),
                snippet,
                embedding_model: MODEL_NAME.to_string(),
                vector,
            });
        }
        self.storage
            .vectors()
            .upsert_attachment_chunks(&row.id, &vrows)?;
        self.set_status(&row.id, "indexed").await?;
        Ok(true)
    }

    async fn set_status(&self, attachment_id: &str, status: &str) -> AppResult<()> {
        sqlx::query("UPDATE attachments SET embedding_att_status = ? WHERE id = ?")
            .bind(status)
            .bind(attachment_id)
            .execute(self.storage.db().pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extraction::ExtractionService;

    #[tokio::test]
    async fn indexes_extracted_attachment_into_vectors_and_fts() {
        let storage = StorageFacade::open_in_memory().await.unwrap();
        seed(&storage, "acc", "m1").await;

        // Write + extract a text attachment first (drives the FTS trigger).
        let w = storage
            .blobs()
            .write_attachment(
                "acc",
                "m1",
                2026,
                6,
                "contract.txt",
                b"master services agreement contract terms",
            )
            .await
            .unwrap();
        let att = crate::util::new_uuid();
        sqlx::query(
            "INSERT INTO attachments (id, mail_id, account_id, filename, content_type, size_bytes, \
             downloaded, local_path, is_inline, extraction_status, created_at) \
             VALUES (?, 'm1', 'acc', 'contract.txt', 'text/plain', 40, 1, ?, 0, 'pending', 0)",
        )
        .bind(&att)
        .bind(&w.relative_path)
        .execute(storage.db().pool())
        .await
        .unwrap();

        ExtractionService::new(storage.clone())
            .extract_one(&att)
            .await
            .unwrap();

        // FTS got filled by the migration-012 trigger on status → indexed.
        let (fts_hits,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM attachments_fts WHERE attachments_fts MATCH 'contract'",
        )
        .fetch_one(storage.db().pool())
        .await
        .unwrap();
        assert!(fts_hits >= 1, "FTS should contain the contract text");

        // Now embed into vectors.
        let indexer =
            AttachmentIndexer::new(storage.clone(), crate::embedding::Embedder::offline());
        let stats = indexer.index_extracted_batch(10).await.unwrap();
        assert_eq!(stats.indexed, 1);
        assert_eq!(stats.remaining, 0);

        // A vector row prefixed "{att}:" exists.
        assert!(storage.vectors().stats().unwrap().total_vectors >= 1);
        let (status,): (String,) =
            sqlx::query_as("SELECT embedding_att_status FROM attachments WHERE id = ?")
                .bind(&att)
                .fetch_one(storage.db().pool())
                .await
                .unwrap();
        assert_eq!(status, "indexed");
    }

    #[tokio::test]
    async fn skipped_extraction_propagates_to_embedding_status() {
        let storage = StorageFacade::open_in_memory().await.unwrap();
        seed(&storage, "acc", "m1").await;
        let att = crate::util::new_uuid();
        sqlx::query(
            "INSERT INTO attachments (id, mail_id, account_id, filename, content_type, size_bytes, \
             downloaded, local_path, is_inline, extraction_status, embedding_att_status, created_at) \
             VALUES (?, 'm1', 'acc', 'clip.mp4', 'video/mp4', 100, 1, 'acc/x/clip.mp4', 0, 'skipped', 'pending', 0)",
        )
        .bind(&att)
        .execute(storage.db().pool())
        .await
        .unwrap();

        let indexer =
            AttachmentIndexer::new(storage.clone(), crate::embedding::Embedder::offline());
        indexer.index_extracted_batch(10).await.unwrap();

        let (status,): (String,) =
            sqlx::query_as("SELECT embedding_att_status FROM attachments WHERE id = ?")
                .bind(&att)
                .fetch_one(storage.db().pool())
                .await
                .unwrap();
        assert_eq!(status, "skipped");
        assert_eq!(storage.vectors().stats().unwrap().total_vectors, 0);
    }

    async fn seed(storage: &StorageFacade, acc: &str, mail: &str) {
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, created_at, updated_at) \
             VALUES (?, 'a@x.com', 'A', 'imap', 'slate', 'W', 0, 0)",
        )
        .bind(acc)
        .execute(storage.db().pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, from_email, to_addrs, date_sent, date_received, subject, created_at, updated_at) \
             VALUES (?, ?, ?, 'a@x.com', '[]', 1000, 1000, 'S', 0, 0)",
        )
        .bind(mail)
        .bind(acc)
        .bind(format!("<{mail}@x>"))
        .execute(storage.db().pool())
        .await
        .unwrap();
    }
}
