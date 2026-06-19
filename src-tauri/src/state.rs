//! Process-wide application state (03 §3).
//!
//! Injected as Tauri `State<AppState>` and shared by every command. It is cheap
//! to `Clone` (every heavy field is behind an `Arc` or is itself an `Arc`-backed
//! handle), so background tasks — the sync scheduler, backfill, parse worker —
//! each hold their own clone without a second source of truth.
//!
//! T019 folded the bare `db` + `vectors` fields into a single [`StorageFacade`]
//! (SQLite = authoritative, LanceDB-style vectors = derived, blobs = on disk).

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::account::oauth::PendingOAuth;
use crate::ai::audit::AuditLogger;
use crate::ai::fallback::FallbackRouter;
use crate::ai::pipeline::notifier::DraftNotifier;
use crate::ai::pipeline::worker::{E2PipelineJob, PipelineQueue};
use crate::ai::pipeline::{PIPELINE_GLOBAL_CONCURRENCY, PIPELINE_PER_ACCOUNT_CONCURRENCY};
use crate::ai::AiRegistry;
use crate::config::Paths;
use crate::embedding::queue::{EmbedJob, EmbedQueue};
use crate::embedding::Embedder;
use crate::error::AppResult;
use crate::events::Emitter;
use crate::identity::PendingIdentitySignin;
use crate::keychain::Keychain;
use crate::net::Net;
use crate::sanitize::Sanitizer;
use crate::send::SendQueue;
use crate::storage::facade::StorageFacade;
use crate::types::RawMail;

/// Bounded capacity of the ingest channel (fetch tasks → parse worker, T023).
/// Back-pressure here naturally throttles fetch when parsing falls behind.
const INGEST_CHANNEL_CAP: usize = 512;

/// Serialises concurrent OAuth refreshes per account so a refresh-token's
/// one-time-use quota is never double-spent (T018 §6).
pub type RefreshGuards = Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub storage: StorageFacade,
    pub keychain: Keychain,
    pub paths: Paths,
    /// HTML sanitiser, built once at boot and reused (T027 §6).
    pub sanitizer: Arc<Sanitizer>,
    /// Typed Tauri event emitter (T024).
    pub events: Emitter,
    /// Network transports behind their test seams (T014/T015/T021).
    pub net: Net,
    /// Transient PKCE/state for an in-flight OAuth grant (T015 §6).
    pub oauth: Arc<Mutex<Option<PendingOAuth>>>,
    /// Transient PKCE/state for an in-flight SeekerMail ID sign-in (loopback OIDC,
    /// T121/A6). Independent of the mailbox `oauth` grant above.
    pub identity_oauth: Arc<Mutex<Option<PendingIdentitySignin>>>,
    /// True while any account is backfilling — auto attachment downloads back off
    /// to 1 concurrent while set (T025 §6).
    pub backfill_active: Arc<AtomicBool>,
    /// Per-account refresh mutex map (T018).
    pub refresh_guards: RefreshGuards,
    /// Sender half of the ingest channel; cloned into each fetch task.
    pub mail_tx: mpsc::Sender<RawMail>,
    /// Local text→vector embedder (T030). Offline-deterministic by default; the
    /// real bge-m3 ONNX runtime is wired with `--features local-embed`.
    pub embedder: Embedder,
    /// Producer handle for the B3 embedding queue (T031); cloned into the ingest
    /// hot path so freshly persisted mails are enqueued for vectorisation.
    pub embed_queue: EmbedQueue,
    /// In-memory registry of cancellable pending sends (T043).
    pub send_queue: SendQueue,
    /// BYO AI provider router (T058). Adapters register at setup; `resolve()`
    /// routes `(account, capability)` and enforces `daily_query_limit`.
    pub ai: AiRegistry,
    /// F5 degradation layer (T067): cooldowns, backup-chain traversal,
    /// E3→E2 downgrade, hold queue. D/E engines call `fallback.invoke()`.
    pub fallback: FallbackRouter,
    /// E7 audit-log writer (T088): append-only `ai_decisions` service shared
    /// by every E-mode pipeline and the draft commands.
    pub audit: AuditLogger,
    /// Global E2/E3 generation concurrency cap — 4 permits (T082, F_E2 §4.6).
    pub e2_semaphore: Arc<tokio::sync::Semaphore>,
    /// Per-account generation semaphores (2 permits each), created lazily via
    /// [`AppState::e2_account_sem`].
    pub e2_account_sems: Arc<Mutex<HashMap<String, Arc<tokio::sync::Semaphore>>>>,
    /// Producer handle for the AI pipeline queue (T082); the parse worker
    /// enqueues one job per freshly inserted inbound mail.
    pub pipeline_queue: PipelineQueue,
    /// Throttled E2 draft OS notifier (T083). The platform sender closure is
    /// installed by `lib.rs` once the app handle exists.
    pub notifier: Arc<DraftNotifier>,
}

impl AppState {
    /// Connect storage, run migrations, build the sanitiser, and assemble shared
    /// state. Returns the state plus the receiver half of the ingest channel,
    /// which `run()` hands to the parse worker (T023).
    pub async fn bootstrap(
        paths: Paths,
        events: Emitter,
    ) -> AppResult<(
        Self,
        mpsc::Receiver<RawMail>,
        mpsc::Receiver<EmbedJob>,
        mpsc::Receiver<E2PipelineJob>,
    )> {
        let storage = StorageFacade::open(&paths).await?;
        tracing::info!(
            event = "storage_ready",
            "database, vectors, and blob store ready"
        );

        let (mail_tx, mail_rx) = mpsc::channel(INGEST_CHANNEL_CAP);
        let embedder = Embedder::load(&paths);
        if embedder.is_offline() {
            // Surface "real model not active" once at boot (09 §4 Recoverable). The
            // pipeline still works on the deterministic offline backend.
            events.gte_error(
                "model",
                "running on the offline embedder; semantic results are approximate",
            );
        }
        let (embed_queue, embed_rx) = EmbedQueue::new();
        let ai = AiRegistry::new(storage.db().clone());
        let fallback = FallbackRouter::new(ai.clone(), storage.db().clone(), events.clone());
        let audit = AuditLogger::new(storage.db().clone());
        let (pipeline_queue, pipeline_rx) = PipelineQueue::new();

        let state = Self {
            storage,
            keychain: Keychain::new(),
            paths,
            sanitizer: Arc::new(Sanitizer::new()),
            events,
            net: Net::resolve(),
            oauth: Arc::new(Mutex::new(None)),
            identity_oauth: Arc::new(Mutex::new(None)),
            backfill_active: Arc::new(AtomicBool::new(false)),
            refresh_guards: Arc::new(Mutex::new(HashMap::new())),
            mail_tx,
            embedder,
            embed_queue,
            send_queue: SendQueue::new(),
            ai,
            fallback,
            audit,
            e2_semaphore: Arc::new(tokio::sync::Semaphore::new(PIPELINE_GLOBAL_CONCURRENCY)),
            e2_account_sems: Arc::new(Mutex::new(HashMap::new())),
            pipeline_queue,
            notifier: Arc::new(DraftNotifier::new()),
        };
        Ok((state, mail_rx, embed_rx, pipeline_rx))
    }

    /// Acquire (creating on first use) the refresh mutex for one account (T018).
    pub fn refresh_lock(&self, account_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut map = self.refresh_guards.lock().expect("refresh_guards poisoned");
        map.entry(account_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    /// The per-account E2/E3 generation semaphore (2 permits), created on
    /// first use (T082, F_E2 §4.6).
    pub fn e2_account_sem(&self, account_id: &str) -> Arc<tokio::sync::Semaphore> {
        let mut map = self
            .e2_account_sems
            .lock()
            .expect("e2_account_sems poisoned");
        map.entry(account_id.to_string())
            .or_insert_with(|| {
                Arc::new(tokio::sync::Semaphore::new(
                    PIPELINE_PER_ACCOUNT_CONCURRENCY,
                ))
            })
            .clone()
    }

    /// In-memory state for unit/integration tests: temp SQLite, no-op emitter,
    /// offline transports. The ingest receiver is returned so callers can drain
    /// or drop it.
    #[cfg(test)]
    pub async fn test_state() -> (Self, mpsc::Receiver<RawMail>) {
        Self::test_state_with_net(Net::resolve()).await
    }

    /// Like [`Self::test_state`] but with a caller-supplied [`Net`]. Inject
    /// `crate::net::fakes::fake_net(...)` to exercise the transport **success**
    /// paths (sync / sampler / connection probe / OAuth refresh) that the
    /// default offline adapters can only fail.
    #[cfg(test)]
    pub async fn test_state_with_net(net: Net) -> (Self, mpsc::Receiver<RawMail>) {
        let storage = StorageFacade::open_in_memory().await.expect("test storage");
        let (mail_tx, mail_rx) = mpsc::channel(INGEST_CHANNEL_CAP);
        let paths = Paths::resolve().expect("paths");
        let embedder = Embedder::load(&paths);
        // Tests drive the queue directly; the worker isn't started, so the receiver
        // is dropped (sends just fail closed, which the catch-up poll covers).
        let (embed_queue, _embed_rx) = EmbedQueue::new();
        let ai = AiRegistry::new(storage.db().clone());
        let events = Emitter::noop();
        let fallback = FallbackRouter::new(ai.clone(), storage.db().clone(), events.clone());
        let audit = AuditLogger::new(storage.db().clone());
        // Same deal for the AI pipeline queue: tests call the pipeline fns
        // directly; enqueues fail closed.
        let (pipeline_queue, _pipeline_rx) = PipelineQueue::new();
        let state = Self {
            storage,
            keychain: Keychain::new(),
            paths,
            sanitizer: Arc::new(Sanitizer::new()),
            events,
            net,
            oauth: Arc::new(Mutex::new(None)),
            identity_oauth: Arc::new(Mutex::new(None)),
            backfill_active: Arc::new(AtomicBool::new(false)),
            refresh_guards: Arc::new(Mutex::new(HashMap::new())),
            mail_tx,
            embedder,
            embed_queue,
            send_queue: SendQueue::new(),
            ai,
            fallback,
            audit,
            e2_semaphore: Arc::new(tokio::sync::Semaphore::new(PIPELINE_GLOBAL_CONCURRENCY)),
            e2_account_sems: Arc::new(Mutex::new(HashMap::new())),
            pipeline_queue,
            notifier: Arc::new(DraftNotifier::new()),
        };
        (state, mail_rx)
    }
}
