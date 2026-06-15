//! Attachment extraction + index commands (T108/T109).
//!
//! Thin `#[tauri::command]` wrappers over [`crate::extraction`]. Both commands
//! kick off a background task and return immediately; progress is pushed over the
//! `extraction:progress` and `attachment_index:progress` events. A single-flight
//! guard per command keeps a second click from launching a parallel sweep.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::State;

use crate::error::IpcError;
use crate::extraction::index::AttachmentIndexer;
use crate::extraction::ExtractionService;
use crate::state::AppState;
use crate::types::{AttachmentIndexBuildStatus, ExtractionBatchStarted};

/// Rows pulled per extraction batch in the backfill loop.
const EXTRACTION_BACKFILL_BATCH: usize = 50;
/// Rows pulled per indexing batch in the build loop.
const INDEX_BUILD_BATCH: usize = 50;

/// Single-flight guards (one sweep of each kind at a time).
static EXTRACTION_BACKFILL_ACTIVE: AtomicBool = AtomicBool::new(false);
static INDEX_BUILD_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Start a background backfill that extracts every downloaded-but-unindexed
/// attachment. Returns the pending count and starts the sweep; progress arrives
/// via `extraction:progress`. Calling again while a sweep runs is a no-op that
/// still reports the current pending count (T108 §3e).
#[tauri::command]
pub async fn start_attachment_extraction_backfill(
    state: State<'_, AppState>,
) -> Result<ExtractionBatchStarted, IpcError> {
    let svc = ExtractionService::from_state(&state);
    let pending = svc.pending_count().await.map_err(IpcError::from)?;

    if EXTRACTION_BACKFILL_ACTIVE.swap(true, Ordering::SeqCst) {
        // Already sweeping — report the live pending count, don't double-start.
        return Ok(ExtractionBatchStarted {
            pending_count: pending,
        });
    }

    let app = (*state).clone();
    tauri::async_runtime::spawn(async move {
        run_extraction_backfill(app, pending as u64).await;
        EXTRACTION_BACKFILL_ACTIVE.store(false, Ordering::SeqCst);
    });

    Ok(ExtractionBatchStarted {
        pending_count: pending,
    })
}

async fn run_extraction_backfill(state: AppState, total: u64) {
    let svc = ExtractionService::from_state(&state);
    let (mut processed, mut indexed, mut skipped, mut errored) = (0u64, 0u64, 0u64, 0u64);
    loop {
        match svc.extract_pending_batch(EXTRACTION_BACKFILL_BATCH).await {
            Ok(stats) => {
                let done = (stats.indexed + stats.skipped + stats.error) as u64;
                if done == 0 {
                    break;
                }
                processed += done;
                indexed += stats.indexed as u64;
                skipped += stats.skipped as u64;
                errored += stats.error as u64;
                state.events.extraction_progress(
                    processed,
                    total.max(processed),
                    indexed,
                    skipped,
                    errored,
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "attachment extraction backfill batch failed");
                break;
            }
        }
    }
    tracing::info!(
        event = "extraction_backfill_done",
        processed,
        indexed,
        skipped,
        errored,
        "attachment extraction backfill finished"
    );
}

/// Build the attachment search index end-to-end (T109 §3d): first extract any
/// pending text, then chunk + embed the freshly `indexed` rows into the vector
/// store and FTS. Returns the pending count and starts the build; progress
/// arrives via `attachment_index:progress`.
#[tauri::command]
pub async fn build_attachment_index(
    state: State<'_, AppState>,
) -> Result<AttachmentIndexBuildStatus, IpcError> {
    let svc = ExtractionService::from_state(&state);
    let total_pending = svc.pending_count().await.map_err(IpcError::from)?;

    if INDEX_BUILD_ACTIVE.swap(true, Ordering::SeqCst) {
        return Ok(AttachmentIndexBuildStatus {
            total_pending,
            started: false,
        });
    }

    let app = (*state).clone();
    tauri::async_runtime::spawn(async move {
        run_attachment_index_build(app).await;
        INDEX_BUILD_ACTIVE.store(false, Ordering::SeqCst);
    });

    Ok(AttachmentIndexBuildStatus {
        total_pending,
        started: true,
    })
}

async fn run_attachment_index_build(state: AppState) {
    let start = std::time::Instant::now();
    let svc = ExtractionService::from_state(&state);
    let indexer = AttachmentIndexer::from_state(&state);

    // Phase 1 — extract all pending attachment text.
    loop {
        match svc.extract_pending_batch(EXTRACTION_BACKFILL_BATCH).await {
            Ok(stats) if (stats.indexed + stats.skipped + stats.error) > 0 => {}
            Ok(_) => break,
            Err(e) => {
                tracing::warn!(error = %e, "attachment index: extraction phase failed");
                break;
            }
        }
    }

    // Phase 2 — chunk + embed the indexed rows into FTS + vectors.
    let mut indexed_total: u64 = 0;
    loop {
        match indexer.index_extracted_batch(INDEX_BUILD_BATCH).await {
            Ok(stats) if stats.indexed > 0 => {
                indexed_total += stats.indexed as u64;
                state.events.attachment_index_progress(
                    indexed_total,
                    indexed_total + stats.remaining as u64,
                    start.elapsed().as_millis() as u64,
                );
            }
            Ok(_) => break,
            Err(e) => {
                tracing::warn!(error = %e, "attachment index: embedding phase failed");
                break;
            }
        }
    }
    tracing::info!(
        event = "attachment_index_build_done",
        indexed = indexed_total,
        elapsed_ms = start.elapsed().as_millis() as u64,
        "attachment index build finished"
    );
}
