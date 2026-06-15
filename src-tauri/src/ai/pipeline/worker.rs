//! Pipeline ingest queue + background worker (T082 §3, T083 §3).
//!
//! The parse worker (T023) `try_send`s one [`E2PipelineJob`] per freshly
//! *inserted* inbound mail. This worker consumes the bounded queue and
//! dispatches by authorization route: `Semi` → E2 pipeline, `Full` → E3
//! pipeline, anything else → skip. Failures are `warn`-logged and surfaced as
//! `pipeline:error` events — one bad mail never stalls the queue.
//!
//! Batch notification (T083): the worker counts E2 drafts created per account
//! while draining; whenever the channel goes momentarily idle, it flushes one
//! merged OS notification per account through the throttled [`DraftNotifier`]
//! ("N AI drafts ready for review" — counts only, never mail content).

use std::collections::HashMap;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::ai::settings::{resolve_auth_route, AuthRouteDecision};
use crate::state::AppState;

/// Bounded queue capacity (T082 §3, mirroring the T031 ingest architecture).
pub const PIPELINE_QUEUE_CAP: usize = 256;

/// One ingested mail awaiting AI-pipeline dispatch.
#[derive(Debug, Clone)]
pub struct E2PipelineJob {
    pub mail_id: String,
    pub account_id: String,
}

/// Producer handle. Cheap to `Clone`; lives in [`AppState`].
#[derive(Clone)]
pub struct PipelineQueue {
    tx: mpsc::Sender<E2PipelineJob>,
}

impl PipelineQueue {
    /// Create the queue + the receiver half handed to [`start_pipeline_worker`].
    pub fn new() -> (Self, mpsc::Receiver<E2PipelineJob>) {
        let (tx, rx) = mpsc::channel(PIPELINE_QUEUE_CAP);
        (Self { tx }, rx)
    }

    /// Non-blocking enqueue. A full channel is non-fatal: the mail simply gets
    /// no automatic draft (warn-logged); the user can still trigger E1.
    pub fn try_enqueue(&self, job: E2PipelineJob) -> bool {
        match self.tx.try_send(job) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(j)) => {
                tracing::warn!(
                    event = "pipeline_queue_full",
                    mail_id = %j.mail_id,
                    "ai pipeline queue full; mail skipped for automatic drafting"
                );
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }
}

/// Process one job: route by auth level and run the matching pipeline.
/// Returns `true` when an E2 review draft was created (the notifier counts
/// those; E3 outcomes notify through their own toast/undo surface).
async fn process_job(state: &AppState, job: &E2PipelineJob) -> crate::error::AppResult<bool> {
    // I3 proactive-query pre-step (T095): if a query is raised, the mail is
    // suspended and the E1/E2/E3 chain does not run for it now — it resumes when
    // the user answers (T096).
    if let super::i3_stage::I3Outcome::Suspended { .. } =
        super::i3_stage::run_i3_detection(state, &job.mail_id, &job.account_id).await?
    {
        return Ok(false);
    }

    let route = match resolve_auth_route(state.storage.db(), &job.account_id).await {
        Ok(route) => route,
        // No settings row = AI not configured for the account → skip quietly.
        Err(crate::error::AppError::NotFound) => return Ok(false),
        Err(e) => return Err(e),
    };
    match route {
        AuthRouteDecision::Semi => {
            let draft =
                super::e2_pipeline::run_e2_for_mail(state, &job.mail_id, &job.account_id).await?;
            Ok(draft.is_some())
        }
        AuthRouteDecision::Full => {
            super::e3_pipeline::run_e3_for_mail(state, &job.mail_id, &job.account_id).await?;
            Ok(false)
        }
        AuthRouteDecision::Manual | AuthRouteDecision::Disabled => Ok(false),
    }
}

/// Spawn the pipeline worker. Runs until the channel closes (every
/// [`PipelineQueue`] clone dropped — i.e. app shutdown), so it exits cleanly
/// without a separate shutdown signal.
pub fn start_pipeline_worker(
    mut rx: mpsc::Receiver<E2PipelineJob>,
    state: AppState,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        // E2 drafts created per account in the current drain burst.
        let mut batch_counts: HashMap<String, usize> = HashMap::new();
        while let Some(job) = rx.recv().await {
            match process_job(&state, &job).await {
                Ok(true) => {
                    *batch_counts.entry(job.account_id.clone()).or_insert(0) += 1;
                }
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(
                        event = "pipeline_job_failed",
                        mail_id = %job.mail_id,
                        account_id = %job.account_id,
                        code = e.code().as_wire(),
                        "ai pipeline job failed; continuing with the next mail"
                    );
                    state
                        .events
                        .pipeline_error(&job.mail_id, e.code().as_wire());
                }
            }

            // Channel momentarily idle → flush the merged notifications
            // (one per account, throttled inside the notifier, T083 §3).
            if rx.is_empty() && !batch_counts.is_empty() {
                for (account_id, count) in batch_counts.drain() {
                    state.notifier.notify_if_needed(&account_id, count);
                }
            }
        }
        tracing::info!(event = "pipeline_worker_stopped", "pipeline queue closed");
    })
}
