//! B3 embedding queue + worker (T031, 03 §15).
//!
//! A bounded `mpsc` channel feeds a **single** background worker (CPU-bound work is
//! serialised on purpose). The worker batches jobs, chunks each mail's body,
//! embeds every chunk in one [`Embedder::embed_batch_blocking`] call, writes one
//! [`VectorRow`] per chunk to the [`VectorStore`], and advances each mail's
//! `embedding_status`. Failures retry up to [`EMBED_MAX_RETRY`] times (30 s apart)
//! before the mail is marked `error` and a `gte:error` event fires.
//!
//! Two ingress paths keep the index complete:
//!   * **Hot path** — the parse worker (T023) `try_send`s each freshly persisted
//!     mail. A full channel is non-fatal: the mail simply stays `pending`.
//!   * **Catch-up path** — when the channel is idle the worker polls
//!     `MailRepo::next_embedding_batch`, recovering anything dropped by a full
//!     channel or left over from a previous run (e.g. after a restart).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::embedding::chunker::chunk_mail;
use crate::embedding::MODEL_NAME;
use crate::state::AppState;
use crate::storage::MailRepo;
use crate::util::now_unix;
use crate::vector::VectorRow;

/// Bounded channel capacity (back-pressure; T031 §6).
pub const EMBED_CHANNEL_CAP: usize = 1024;
/// Chunks-per-mails batched into one embed call (F_B3 §4.3).
pub const EMBED_BATCH_SIZE: usize = 32;
/// CPU yield between batches to hold the soft ~30 % CPU cap (F_B3 §4.3).
pub const EMBED_CPU_YIELD_MS: u64 = 50;
/// Idle wait before the worker falls back to a catch-up DB poll.
pub const EMBED_IDLE_POLL_SECS: u64 = 10;
/// Max embed attempts before a mail is marked `error` (F_B3 §4.4).
pub const EMBED_MAX_RETRY: u8 = 3;
/// Delay between retries (F_B3 §4.4).
pub const EMBED_RETRY_DELAY_SECS: u64 = 30;

/// One mail awaiting embedding. Carries the body so the worker chunks without a
/// second DB round-trip.
#[derive(Debug, Clone)]
pub struct EmbedJob {
    pub mail_id: String,
    pub account_id: String,
    pub from_email: String,
    pub date_sent: i64,
    pub subject: String,
    pub snippet: String,
    pub body_text: String,
    /// Retry counter; 0 on first enqueue.
    pub retry: u8,
}

/// The producer handle. Cheap to `Clone`; lives in [`AppState`].
#[derive(Clone)]
pub struct EmbedQueue {
    tx: mpsc::Sender<EmbedJob>,
    pause: Arc<AtomicBool>,
}

impl EmbedQueue {
    /// Create the queue + the receiver half handed to [`start_worker`].
    pub fn new() -> (Self, mpsc::Receiver<EmbedJob>) {
        let (tx, rx) = mpsc::channel(EMBED_CHANNEL_CAP);
        (
            Self {
                tx,
                pause: Arc::new(AtomicBool::new(false)),
            },
            rx,
        )
    }

    /// Non-blocking enqueue. Returns `false` (and warn-logs) when the channel is
    /// full — the caller leaves the mail `pending` for the catch-up poll to claim.
    pub fn try_send(&self, job: EmbedJob) -> bool {
        match self.tx.try_send(job) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(j)) => {
                tracing::warn!(mail_id = %j.mail_id, "embed queue full; mail stays pending");
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    /// Pause/resume the worker (no IPC yet; H1 settings wires this in v0.5).
    pub fn set_paused(&self, paused: bool) {
        self.pause.store(paused, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.pause.load(Ordering::Relaxed)
    }

    fn pause_flag(&self) -> Arc<AtomicBool> {
        self.pause.clone()
    }
}

/// Spawn the single embedding worker. It runs until the channel closes (every
/// [`EmbedQueue`] clone dropped — i.e. app shutdown), finishing the current batch
/// first. Returns its `JoinHandle`.
pub fn start_worker(mut rx: mpsc::Receiver<EmbedJob>, state: AppState) -> JoinHandle<()> {
    let pause = state.embed_queue.pause_flag();
    tokio::spawn(async move {
        let session_start = Instant::now();
        let mut session_indexed: u64 = 0;
        let mut idle_announced = true; // nothing pending until we see work

        loop {
            if pause.load(Ordering::Relaxed) {
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            }

            // Gather a batch: block up to EMBED_IDLE_POLL_SECS for the first job,
            // then greedily drain whatever else is already queued.
            let batch =
                match tokio::time::timeout(Duration::from_secs(EMBED_IDLE_POLL_SECS), rx.recv())
                    .await
                {
                    Ok(Some(first)) => {
                        let mut b = Vec::with_capacity(EMBED_BATCH_SIZE);
                        b.push(first);
                        while b.len() < EMBED_BATCH_SIZE {
                            match rx.try_recv() {
                                Ok(j) => b.push(j),
                                Err(_) => break,
                            }
                        }
                        b
                    }
                    Ok(None) => {
                        tracing::info!(event = "embed_worker_stopped", "embed queue closed");
                        break;
                    }
                    Err(_) => {
                        // Idle → catch-up poll for stragglers.
                        match MailRepo::new(state.storage.db())
                            .next_embedding_batch(EMBED_BATCH_SIZE as i64)
                            .await
                        {
                            Ok(jobs) if !jobs.is_empty() => jobs,
                            Ok(_) => {
                                if !idle_announced && session_indexed > 0 {
                                    state.events.gte_finished(
                                        session_indexed,
                                        session_start.elapsed().as_millis() as u64,
                                    );
                                    idle_announced = true;
                                }
                                continue;
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "embed catch-up poll failed");
                                continue;
                            }
                        }
                    }
                };

            idle_announced = false;
            let indexed = process_batch(&state, &batch).await;
            session_indexed += indexed;

            let total_pending = MailRepo::new(state.storage.db())
                .count_pending_embeddings()
                .await
                .unwrap_or(0)
                .max(0) as u64;
            let secs = session_start.elapsed().as_secs_f32().max(0.001);
            state.events.gte_progress(
                session_indexed,
                total_pending,
                session_indexed as f32 / secs,
            );

            // Yield CPU between batches to respect the soft cap.
            tokio::time::sleep(Duration::from_millis(EMBED_CPU_YIELD_MS)).await;
        }
    })
}

/// Embed one batch. Returns the number of mails newly marked `indexed`. Embed /
/// upsert failures are retried per-mail (or marked `error`) internally, so the
/// worker loop never has to special-case them. `pub(crate)` so the reindex task
/// (T053) can drive batches synchronously while the worker is paused.
pub(crate) async fn process_batch(state: &AppState, batch: &[EmbedJob]) -> u64 {
    let repo = MailRepo::new(state.storage.db());

    // Partition into "skip" (no body / no chunks) and "work" (mail + its chunks).
    let mut skipped: Vec<String> = Vec::new();
    let mut work: Vec<(&EmbedJob, Vec<String>)> = Vec::new();
    for job in batch {
        if job.body_text.trim().is_empty() {
            skipped.push(job.mail_id.clone());
            continue;
        }
        let chunks = chunk_mail(&job.body_text);
        if chunks.is_empty() {
            skipped.push(job.mail_id.clone());
        } else {
            work.push((job, chunks));
        }
    }

    if !skipped.is_empty() {
        if let Err(e) = repo
            .update_embedding_status(&skipped, "skipped", None, None)
            .await
        {
            tracing::warn!(error = %e, "mark skipped failed");
        }
    }
    if work.is_empty() {
        return 0;
    }

    // One embed call for every chunk in the batch.
    let all_texts: Vec<String> = work.iter().flat_map(|(_, c)| c.clone()).collect();
    let vectors = match state.embedder.embed_batch_blocking(all_texts).await {
        Ok(v) => v,
        Err(e) => {
            for (job, _) in &work {
                reschedule_or_fail(state, job, &e.to_string()).await;
            }
            return 0;
        }
    };

    // Build one VectorRow per chunk (chunk_id = "{mail_id}:{chunk_index}").
    let now = now_unix();
    let mut rows: Vec<VectorRow> = Vec::with_capacity(vectors.len());
    let mut indexed_ids: Vec<String> = Vec::with_capacity(work.len());
    let mut cursor = 0usize;
    for (job, chunks) in &work {
        for (idx, _chunk) in chunks.iter().enumerate() {
            let vector = vectors[cursor].clone();
            cursor += 1;
            rows.push(VectorRow {
                chunk_id: format!("{}:{}", job.mail_id, idx),
                mail_id: job.mail_id.clone(),
                chunk_index: idx as i32,
                account_id: job.account_id.clone(),
                from_email: job.from_email.clone(),
                date_sent: job.date_sent,
                subject: job.subject.clone(),
                snippet: job.snippet.clone(),
                embedding_model: MODEL_NAME.to_string(),
                vector,
            });
        }
        indexed_ids.push(job.mail_id.clone());
    }

    if let Err(e) = state.storage.vectors().upsert(&rows) {
        for (job, _) in &work {
            reschedule_or_fail(state, job, &e.to_string()).await;
        }
        return 0;
    }

    if let Err(e) = repo
        .update_embedding_status(&indexed_ids, "indexed", Some(MODEL_NAME), Some(now))
        .await
    {
        tracing::warn!(error = %e, "mark indexed failed");
        return 0;
    }
    indexed_ids.len() as u64
}

/// Re-enqueue a failed job after a delay, or mark it `error` once retries run out.
async fn reschedule_or_fail(state: &AppState, job: &EmbedJob, reason: &str) {
    if job.retry + 1 >= EMBED_MAX_RETRY {
        if let Err(e) = MailRepo::new(state.storage.db())
            .update_embedding_status(std::slice::from_ref(&job.mail_id), "error", None, None)
            .await
        {
            tracing::warn!(error = %e, "mark error failed");
        }
        state.events.gte_error(&job.mail_id, reason);
        tracing::warn!(mail_id = %job.mail_id, retries = job.retry + 1, "embedding gave up");
    } else {
        let queue = state.embed_queue.clone();
        let mut next = job.clone();
        next.retry += 1;
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(EMBED_RETRY_DELAY_SECS)).await;
            queue.try_send(next);
        });
    }
}

/// Build the [`EmbedJob`] for a freshly persisted mail (used by the ingest hot
/// path, T023 → T031). Returns `None` when the body is empty (nothing to embed).
pub fn job_from_summary(
    summary: &crate::types::MailSummary,
    body_text: Option<&str>,
) -> Option<EmbedJob> {
    let body = body_text?.to_string();
    if body.trim().is_empty() {
        return None;
    }
    Some(EmbedJob {
        mail_id: summary.id.clone(),
        account_id: summary.account_id.clone(),
        from_email: summary.from_email.clone(),
        date_sent: summary.date_sent,
        subject: summary.subject.clone(),
        snippet: summary.snippet.clone().unwrap_or_default(),
        body_text: body,
        retry: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn end_to_end_indexes_pending_mail() {
        // ingest a mail → catch-up poll path → embedded → status=indexed + vector row.
        let (state, _mail_rx) = AppState::test_state().await;
        seed_account_and_mail(&state, "m1", "Quarterly revenue report attached for review").await;

        // Drive one batch directly (no worker task) via the catch-up source.
        let batch = MailRepo::new(state.storage.db())
            .next_embedding_batch(32)
            .await
            .unwrap();
        assert_eq!(batch.len(), 1);
        let n = process_batch(&state, &batch).await;
        assert_eq!(n, 1);

        // status flipped + vector present.
        let (status,): (String,) =
            sqlx::query_as("SELECT embedding_status FROM mails WHERE id = 'm1'")
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        assert_eq!(status, "indexed");
        assert_eq!(state.storage.vectors().stats().unwrap().total_vectors, 1);
    }

    #[tokio::test]
    async fn empty_body_is_skipped_not_embedded() {
        let (state, _rx) = AppState::test_state().await;
        seed_account_and_mail(&state, "m2", "   ").await;
        let batch = MailRepo::new(state.storage.db())
            .next_embedding_batch(32)
            .await
            .unwrap();
        let n = process_batch(&state, &batch).await;
        assert_eq!(n, 0);
        let (status,): (String,) =
            sqlx::query_as("SELECT embedding_status FROM mails WHERE id = 'm2'")
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        assert_eq!(status, "skipped");
        assert_eq!(state.storage.vectors().stats().unwrap().total_vectors, 0);
    }

    #[tokio::test]
    async fn update_embedding_status_is_idempotent() {
        let (state, _rx) = AppState::test_state().await;
        seed_account_and_mail(&state, "m3", "body").await;
        let repo = MailRepo::new(state.storage.db());
        repo.update_embedding_status(&["m3".into()], "indexed", Some("bge-m3"), Some(now_unix()))
            .await
            .unwrap();
        repo.update_embedding_status(&["m3".into()], "indexed", Some("bge-m3"), Some(now_unix()))
            .await
            .unwrap();
        assert_eq!(repo.count_pending_embeddings().await.unwrap(), 0);
    }

    /// Insert an account + one mail with `embedding_status='pending'`.
    async fn seed_account_and_mail(state: &AppState, mail_id: &str, body: &str) {
        let db = state.storage.db();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, created_at, updated_at) \
             VALUES ('acc', 'a@x.com', 'A', 'imap', 'slate', 'W', 0, 0)",
        )
        .execute(db.pool())
        .await
        .ok();
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, from_email, to_addrs, date_sent, date_received, \
                 body_text, subject, embedding_status, created_at, updated_at) \
             VALUES (?, 'acc', ?, 'a@x.com', '[]', 1000, 1000, ?, 'Subject', 'pending', 0, 0)",
        )
        .bind(mail_id)
        .bind(format!("<{mail_id}@x>"))
        .bind(body)
        .execute(db.pool())
        .await
        .unwrap();
    }
}
