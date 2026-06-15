//! `local_onnx` — the in-process, fully offline generative provider (T063,
//! dev/06 §1, F_F2 §4.3–§4.5).
//!
//! Unlike every other adapter in this directory, `LocalOnnxClient` opens **no
//! socket at all**: the model runs inside this process, so nothing — prompt,
//! completion, or metadata — ever leaves the device. That is why dev/06 §1
//! marks this provider "no disclosure". ADR-0004 is satisfied trivially.
//!
//! ## Backend note (deliberate, mirrors T030's `Embedder`)
//!
//! The card specifies in-process GGUF inference via `llama-cpp-2`. Exactly as
//! the embedding runtime (T030) keeps the heavy `ort` stack behind the
//! OFF-by-default `local-embed` feature, the real llama.cpp runtime here lives
//! behind the OFF-by-default **`local-llm`** feature ([`llama`]). The default
//! build wires a deterministic, dependency-free [`OfflineBackend`] behind the
//! same internal [`GenerativeBackend`] seam, so the full provider lifecycle —
//! model-file discovery, lazy load, single-permit inference, idle unload,
//! streaming — is exercised end-to-end in tests and dev without native libs or
//! a multi-GB model. Enabling `--features local-llm` swaps in the real runtime
//! without touching any caller.
//!
//! Per dev/06 §1, the v0.5 generative model is **manually placed** by the user
//! in the `models/` directory (no auto-download; that is v1.0+ / T068). The
//! file's *presence* gates availability in every build: a missing model is
//! `AI_PROVIDER_UNREACHABLE`, never a silent template fallback.
//!
//! ## Embedding boundary (dev/06 §0 principle 3)
//!
//! This client never touches the `bge-m3` embedding artifacts (`bge-m3.onnx`,
//! `tokenizer.json`, `model.lock.json` — T010/T030). The two stacks share the
//! `models/` directory but are instantiated independently; discovery here
//! explicitly skips the embedding files. `Capability` has no embed variant, so
//! the type system already prevents routing embedding work through this path.
//!
//! ## Documented deviations from the card
//!
//! * `default_model_dir(paths)` / `list_local_generative_models(dir)` take the
//!   resolved location as a parameter instead of re-resolving global state —
//!   consistent with how `Paths` is injected everywhere else, and testable.
//! * `chat_stream()` runs the full completion and then chunks it into
//!   word-group deltas. True token-by-token streaming belongs to the real
//!   runtime and lands with the first `local-llm` integration build.
//! * `health()` checks model-file presence only (the card's "no inference in
//!   the probe" rule); runtime initialization is deferred to the first lazy
//!   load so the probe stays fast.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::stream;
use tokio::sync::{Mutex, Semaphore};

use crate::ai::provider::{AiProviderClient, ChatDeltaStream, ProviderError};
use crate::ai::registry::AccountAiConfig;
#[cfg(not(feature = "local-llm"))]
use crate::ai::types::{Capability, ChatRole};
use crate::ai::types::{
    ChatDelta, ChatRequest, ChatResponse, FinishReason, ProviderHealth, TokenUsage,
};
use crate::config::Paths;
use crate::error::AppResult;
use crate::types::AiProvider;

// ── Tunable constants (no magic numbers inline, T014 §9 convention) ──────────

/// Conservative token budget for small local generative models (card §3,
/// F_F2 §4.3). The context packer (dev/06 §5) treats this as the window.
pub const DEFAULT_CONTEXT_WINDOW: usize = 4096;
/// One in-process inference at a time (F_F2 §4.4 concurrency cap).
const MAX_CONCURRENT_INFERENCE: usize = 1;
/// CPU inference allowance per call (F_F2 §4.4: 180 s — local CPU can be slow).
const INFERENCE_TIMEOUT_SECS: u64 = 180;
/// Unload the model after this much idle time to release RAM (card §3).
const IDLE_UNLOAD_SECS: u64 = 30 * 60;
/// How often the idle watchdog wakes to check (card §6: every 5 minutes).
const IDLE_CHECK_INTERVAL_SECS: u64 = 5 * 60;
/// Whitespace-delimited pieces per streamed delta in `chat_stream`.
const STREAM_PIECES_PER_DELTA: usize = 8;
/// File stem of the embedding model (T010/T030) — never a generative candidate.
const EMBEDDING_MODEL_STEM: &str = "bge-m3";

// ── Model-file discovery (card §3 public path helpers) ───────────────────────

/// One manually-placed generative model file, as shown by the T068 settings UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalModelInfo {
    pub file_name: String,
    pub path: PathBuf,
    pub size_bytes: u64,
}

/// Where generative model files live: the shared `models/` resource directory
/// (same root the embedding artifacts use, T010).
pub fn default_model_dir(paths: &Paths) -> PathBuf {
    paths.models.clone()
}

/// Scan a directory for generative model files (`.gguf` / `.onnx`), skipping
/// the embedding artifacts (dev/06 §0 principle 3). Unreadable directories
/// yield an empty list — "no model installed", never a hard error.
pub fn list_local_generative_models(dir: &Path) -> Vec<LocalModelInfo> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    let mut models: Vec<LocalModelInfo> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() || !is_generative_model_file(&path) {
                return None;
            }
            let file_name = path.file_name()?.to_str()?.to_string();
            let size_bytes = entry.metadata().ok()?.len();
            Some(LocalModelInfo {
                file_name,
                path,
                size_bytes,
            })
        })
        .collect();
    models.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    models
}

/// `.gguf` is always a generative candidate; `.onnx` only when it is not the
/// bge-m3 embedding export. Everything else (tokenizer, lock files) is ignored.
fn is_generative_model_file(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("gguf") => true,
        Some("onnx") => path.file_stem().and_then(|s| s.to_str()) != Some(EMBEDDING_MODEL_STEM),
        _ => false,
    }
}

/// Pick the model file to load. A configured name must match exactly; with no
/// preference, `.gguf` (the wired runtime format) wins over `.onnx` (the
/// card's reserved alternative path), then stable name order.
fn find_model_file_in(dir: &Path, preferred: Option<&str>) -> Option<PathBuf> {
    if let Some(name) = preferred {
        let path = dir.join(name);
        if path.is_file() && is_generative_model_file(&path) {
            return Some(path);
        }
        return None;
    }
    let mut candidates: Vec<PathBuf> = list_local_generative_models(dir)
        .into_iter()
        .map(|m| m.path)
        .collect();
    candidates.sort_by_key(|p| {
        let is_gguf = p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("gguf"));
        (!is_gguf, p.file_name().map(|n| n.to_os_string()))
    });
    candidates.into_iter().next()
}

/// Human label echoed as `model_echo` / `ProviderHealth::model_name`: the file
/// stem of the model on disk (a file name, never content — log-safe, 09 §5).
fn model_label(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_string)
}

// ── Internal generation seam ──────────────────────────────────────────────────

/// The synchronous generation seam both builds implement. Runs inside
/// `tokio::task::spawn_blocking` (card §6), so implementations may block.
trait GenerativeBackend: Send + Sync {
    fn generate(&self, req: &ChatRequest) -> Result<String, ProviderError>;
}

/// A loaded model: the backend plus its disk label. Held behind `Arc` so an
/// in-flight inference survives an idle unload racing it.
struct LoadedModel {
    backend: Box<dyn GenerativeBackend>,
    label: String,
}

/// Lazy-load state shared between the client and its idle watchdog task.
struct ModelSlot {
    /// `None` until first use, and again after an idle unload. The lock is held
    /// across the (blocking-task) load so concurrent callers load exactly once.
    loaded: Mutex<Option<Arc<LoadedModel>>>,
    /// Most recent call into the provider; drives the idle-unload decision.
    last_used: StdMutex<Instant>,
    /// The watchdog is spawned once, on the first successful load.
    watchdog_started: AtomicBool,
    /// How many times a backend was constructed (lifecycle observability; the
    /// lazy-load acceptance tests assert on it).
    loads: AtomicU32,
}

impl ModelSlot {
    fn new() -> Self {
        Self {
            loaded: Mutex::new(None),
            last_used: StdMutex::new(Instant::now()),
            watchdog_started: AtomicBool::new(false),
            loads: AtomicU32::new(0),
        }
    }

    fn touch(&self) {
        *self.last_used.lock().expect("last_used lock poisoned") = Instant::now();
    }

    /// Drop the loaded model if the slot has been idle for `idle_limit`.
    /// `now` is injected so tests can drive the clock instead of sleeping.
    /// Returns true when an unload actually happened.
    async fn unload_if_idle(&self, now: Instant, idle_limit: Duration) -> bool {
        let idle =
            now.saturating_duration_since(*self.last_used.lock().expect("last_used lock poisoned"));
        if idle < idle_limit {
            return false;
        }
        let mut guard = self.loaded.lock().await;
        if guard.take().is_some() {
            tracing::info!(
                event = "local_model_unloaded",
                idle_secs = idle.as_secs(),
                "idle local generative model unloaded to release memory"
            );
            true
        } else {
            false
        }
    }
}

// ── The provider client ───────────────────────────────────────────────────────

/// In-process generative provider (`AiProvider::LocalOnnx`). Zero network:
/// the model file is read from the local `models/` directory and inference
/// runs inside this process on a blocking thread.
pub struct LocalOnnxClient {
    model_dir: PathBuf,
    /// Exact model file name from `account_ai_settings.ai_model`, when set.
    preferred_model: Option<String>,
    context_length: usize,
    /// Single-permit gate: one inference at a time (F_F2 §4.4).
    gate: Semaphore,
    slot: Arc<ModelSlot>,
}

impl LocalOnnxClient {
    /// Singleton constructor for registry registration at boot: serves any
    /// account whose settings select `local_onnx` with no explicit model name.
    pub fn new(paths: &Paths) -> Self {
        Self::with_model_dir(default_model_dir(paths))
    }

    /// Point the client at an explicit model directory (used by tests and by
    /// the T068 custom-path configuration).
    pub fn with_model_dir(model_dir: PathBuf) -> Self {
        Self {
            model_dir,
            preferred_model: None,
            context_length: DEFAULT_CONTEXT_WINDOW,
            gate: Semaphore::new(MAX_CONCURRENT_INFERENCE),
            slot: Arc::new(ModelSlot::new()),
        }
    }

    /// Per-account factory entry point (cross-card convention, T059–T063).
    /// Only `cfg.model` matters here — it selects a specific file in `models/`;
    /// keys and base URLs do not apply to an in-process provider.
    pub fn from_config(cfg: &AccountAiConfig, paths: &Paths) -> AppResult<Arc<Self>> {
        let mut client = Self::new(paths);
        client.preferred_model = cfg.model.clone().filter(|m| !m.trim().is_empty());
        Ok(Arc::new(client))
    }

    fn find_model_file(&self) -> Option<PathBuf> {
        find_model_file_in(&self.model_dir, self.preferred_model.as_deref())
    }

    /// Return the loaded model, constructing the backend on first use. The
    /// slot mutex is held across the load, so concurrent first calls block on
    /// the same load instead of repeating it (card §7 lazy-load criterion).
    async fn get_or_load(&self) -> Result<Arc<LoadedModel>, ProviderError> {
        self.slot.touch();
        let mut guard = self.slot.loaded.lock().await;
        if let Some(model) = guard.as_ref() {
            return Ok(model.clone());
        }

        let path = self
            .find_model_file()
            .ok_or_else(|| ProviderError::Unreachable("local model not installed".into()))?;
        let label = model_label(&path).unwrap_or_else(|| "local-model".into());

        // Backend construction can read a multi-GB file (real runtime), so it
        // runs on the blocking pool — the slot lock is held across the await,
        // which is exactly the single-load guarantee we want.
        let context_length = self.context_length;
        let load_path = path.clone();
        let backend = tokio::task::spawn_blocking(move || load_backend(&load_path, context_length))
            .await
            .map_err(|e| ProviderError::Unreachable(format!("model load task join: {e}")))??;

        let loaded = Arc::new(LoadedModel { backend, label });
        *guard = Some(loaded.clone());
        let loads = self.slot.loads.fetch_add(1, Ordering::SeqCst) + 1;
        tracing::info!(
            event = "local_model_loaded",
            model = %loaded.label,
            load_count = loads,
            "local generative model loaded"
        );
        self.ensure_watchdog();
        Ok(loaded)
    }

    /// Spawn the idle-unload watchdog once (card §6: wake every 5 minutes,
    /// unload after 30 idle minutes). The task holds only a `Weak` reference,
    /// so it exits when the client is dropped.
    fn ensure_watchdog(&self) {
        if self.slot.watchdog_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let slot = Arc::downgrade(&self.slot);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(IDLE_CHECK_INTERVAL_SECS)).await;
                let Some(slot) = slot.upgrade() else { break };
                slot.unload_if_idle(Instant::now(), Duration::from_secs(IDLE_UNLOAD_SECS))
                    .await;
            }
        });
    }
}

#[async_trait]
impl AiProviderClient for LocalOnnxClient {
    /// Note on the embedding boundary (card §6): `Capability` has no embed
    /// variant, so a misrouted embedding call cannot be expressed — the T030
    /// `Embedder` is a separate type with a separate entry point.
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let started = Instant::now();

        let prompt_tokens = estimate_prompt_tokens(&req);
        if prompt_tokens as usize > self.context_length {
            return Err(ProviderError::ContextTooLong);
        }

        let _permit = self.gate.acquire().await.expect("inference gate closed");
        let loaded = self.get_or_load().await?;

        let model_for_task = loaded.clone();
        let gen_req = req.clone();
        let raw = match tokio::time::timeout(
            Duration::from_secs(INFERENCE_TIMEOUT_SECS),
            tokio::task::spawn_blocking(move || model_for_task.backend.generate(&gen_req)),
        )
        .await
        {
            Err(_) => {
                return Err(ProviderError::Unreachable(format!(
                    "local inference timed out after {INFERENCE_TIMEOUT_SECS}s"
                )))
            }
            Ok(Err(join_err)) => {
                return Err(ProviderError::Unreachable(format!(
                    "inference task join: {join_err}"
                )))
            }
            Ok(Ok(result)) => result?,
        };

        let (text, finish) = finalize_completion(raw, &req);
        self.slot.touch();

        let response = ChatResponse {
            usage: TokenUsage {
                prompt_tokens,
                completion_tokens: word_count(&text) as u32,
            },
            model_echo: loaded.label.clone(),
            latency_ms: elapsed_ms(started),
            text,
            finish,
        };
        // Identifiers and numbers only — never prompt/completion text (09 §5),
        // even though this call never left the process.
        tracing::debug!(
            event = "local_chat_complete",
            request_id = %req.request_id,
            capability = req.purpose.as_str(),
            latency_ms = response.latency_ms,
            completion_tokens = response.usage.completion_tokens,
        );
        Ok(response)
    }

    /// Runs the full in-process completion, then replays it as word-group
    /// deltas. With an in-process backend there is no wire latency to hide, so
    /// chunk-after-complete is correct and cheap; true token streaming is the
    /// `local-llm` runtime's follow-on.
    async fn chat_stream(&self, req: ChatRequest) -> Result<ChatDeltaStream, ProviderError> {
        let response = self.chat(req).await?;
        let deltas: Vec<Result<ChatDelta, ProviderError>> = chunk_for_stream(&response.text)
            .into_iter()
            .enumerate()
            .map(|(index, text)| Ok(ChatDelta { text, index }))
            .collect();
        Ok(Box::pin(stream::iter(deltas)))
    }

    /// File-presence probe only (card §3): no model load, no inference, so the
    /// settings screen stays responsive even with a multi-GB model installed.
    async fn health(&self) -> Result<ProviderHealth, ProviderError> {
        let started = Instant::now();
        match self.find_model_file() {
            Some(path) => Ok(ProviderHealth {
                ok: true,
                model_name: model_label(&path),
                latency_ms: elapsed_ms(started),
            }),
            None => Err(ProviderError::Unreachable("model file not found".into())),
        }
    }

    fn id(&self) -> AiProvider {
        AiProvider::LocalOnnx
    }

    fn context_window(&self) -> usize {
        self.context_length
    }
}

/// Settings-screen probe backing `verify_ai_provider`.
///
/// Cross-adapter convention (T059/T060/T062/T063): every adapter module
/// exposes this exact signature and the command layer dispatches on
/// `AiProvider`. `api_key`/`base_url` are accepted for signature parity and
/// ignored — an in-process provider has no endpoint and no key. The check is
/// model-file presence in the default `models/` location (`model`, when
/// non-empty, names the exact file the account is configured to use).
pub async fn probe(
    model: &str,
    _api_key: Option<&str>,
    _base_url: Option<&str>,
) -> Result<ProviderHealth, ProviderError> {
    let started = Instant::now();
    let paths = Paths::resolve()
        .map_err(|e| ProviderError::Unreachable(format!("resolve app data paths: {e}")))?;
    let preferred = {
        let trimmed = model.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    };
    match find_model_file_in(&paths.models, preferred.as_deref()) {
        Some(path) => Ok(ProviderHealth {
            ok: true,
            model_name: model_label(&path),
            latency_ms: elapsed_ms(started),
        }),
        None => Err(ProviderError::Unreachable(
            "local model not installed".into(),
        )),
    }
}

// ── Backend construction (per-build wiring) ───────────────────────────────────

/// Real runtime: GGUF via `llama-cpp-2` (card §6 backend choice). A non-GGUF
/// file is the card's deliberately-unwired ONNX alternative.
#[cfg(feature = "local-llm")]
fn load_backend(
    path: &Path,
    context_length: usize,
) -> Result<Box<dyn GenerativeBackend>, ProviderError> {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("gguf") => {
            Ok(Box::new(llama::GgufBackend::load(path, context_length)?))
        }
        _ => Err(ProviderError::Unreachable(
            "onnx generative runtime is not wired in this build; place a .gguf model in models/"
                .into(),
        )),
    }
}

/// Default build: the deterministic offline backend. The model file still
/// gates availability (callers reach this only after discovery succeeded), so
/// install/uninstall behaves identically across builds.
#[cfg(not(feature = "local-llm"))]
fn load_backend(
    path: &Path,
    context_length: usize,
) -> Result<Box<dyn GenerativeBackend>, ProviderError> {
    let _ = (path, context_length);
    Ok(Box::new(OfflineBackend))
}

// ── Completion post-processing (shared by both backends) ─────────────────────

/// Apply stop sequences, then the `max_tokens` budget (approximated as words —
/// see [`word_count`]), then trim trailing whitespace.
fn finalize_completion(raw: String, req: &ChatRequest) -> (String, FinishReason) {
    let mut text = raw;
    if let Some(cut) = req
        .stop
        .iter()
        .filter_map(|s| {
            if s.is_empty() {
                None
            } else {
                text.find(s.as_str())
            }
        })
        .min()
    {
        text.truncate(cut);
    }
    let mut finish = FinishReason::Stop;
    if let Some(offset) = truncation_offset(&text, req.max_tokens) {
        text.truncate(offset);
        finish = FinishReason::Length;
    }
    let trimmed = text.trim_end().len();
    text.truncate(trimmed);
    (text, finish)
}

/// Byte offset where word number `max_words + 1` begins, or `None` when the
/// text fits the budget. Used to truncate without splitting a word.
fn truncation_offset(text: &str, max_words: u32) -> Option<usize> {
    let mut words = 0u32;
    let mut in_word = false;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            in_word = false;
        } else if !in_word {
            in_word = true;
            if words == max_words {
                return Some(i);
            }
            words += 1;
        }
    }
    None
}

/// Word-count token approximation (≈1 token per whitespace-delimited word).
/// Deliberately simple: this provider has no tokenizer in the default build,
/// and the estimate only drives the context guard and the audit columns.
fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn estimate_prompt_tokens(req: &ChatRequest) -> u32 {
    let mut words = word_count(&req.system);
    for message in &req.messages {
        words += word_count(&message.content);
    }
    words as u32
}

/// Split into word-group chunks whose concatenation is byte-identical to the
/// input (whitespace preserved), so the streamed deltas reassemble exactly.
fn chunk_for_stream(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let pieces: Vec<&str> = text.split_inclusive(|c: char| c.is_whitespace()).collect();
    pieces
        .chunks(STREAM_PIECES_PER_DELTA)
        .map(|chunk| chunk.concat())
        .collect()
}

fn elapsed_ms(started: Instant) -> u32 {
    u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX)
}

// ─────────────────────────────────────────────────────────────────────────────
// Offline backend — deterministic structured generation (default build)
// ─────────────────────────────────────────────────────────────────────────────

/// Dependency-free generation: a pure function of the request content. Each
/// capability gets a structured, extractive English response built from the
/// actual messages (counts, computed stats, quoted excerpts), so the draft and
/// risk pipelines see realistic, stable text in dev and tests — mirroring how
/// the offline embedder gives the GTE pipeline real geometry without a model.
#[cfg(not(feature = "local-llm"))]
struct OfflineBackend;

#[cfg(not(feature = "local-llm"))]
impl GenerativeBackend for OfflineBackend {
    fn generate(&self, req: &ChatRequest) -> Result<String, ProviderError> {
        Ok(match req.purpose {
            Capability::Summarize => compose_summary(req),
            Capability::DraftReply => compose_draft_reply(req),
            Capability::RiskReason => compose_risk_reason(req),
            Capability::StyleProfile => compose_style_profile(req),
        })
    }
}

#[cfg(not(feature = "local-llm"))]
fn compose_summary(req: &ChatRequest) -> String {
    let total_words: usize = req.messages.iter().map(|m| word_count(&m.content)).sum();
    let mut out = format!(
        "Thread summary ({} message{}, about {} words):\n",
        req.messages.len(),
        if req.messages.len() == 1 { "" } else { "s" },
        total_words
    );
    for (i, message) in req.messages.iter().enumerate() {
        out.push_str(&format!(
            "{}. {}: {}\n",
            i + 1,
            message.role.as_str(),
            excerpt(&message.content, 24)
        ));
    }
    out.push_str(
        "Action: review the points above and reply where a response is still outstanding.",
    );
    out
}

#[cfg(not(feature = "local-llm"))]
fn compose_draft_reply(req: &ChatRequest) -> String {
    let topic = last_user_excerpt(req, 18);
    format!(
        "Thank you for your message.\n\n\
         Regarding \"{topic}\": I have reviewed the points raised and will follow up \
         with the requested details shortly. If any item needs priority handling, \
         please let me know and I will address it first.\n\n\
         Best regards"
    )
}

#[cfg(not(feature = "local-llm"))]
fn compose_risk_reason(req: &ChatRequest) -> String {
    let focus = last_user_excerpt(req, 18);
    format!(
        "Risk review (on-device analysis):\n\
         - Messages examined: {}\n\
         - Focus excerpt: \"{focus}\"\n\
         - Recommendation: hold for human review. The local backend performs \
         structural checks only and does not clear sensitive content on its own.",
        req.messages.len()
    )
}

#[cfg(not(feature = "local-llm"))]
fn compose_style_profile(req: &ChatRequest) -> String {
    let samples = req.messages.len().max(1);
    let total_words: usize = req.messages.iter().map(|m| word_count(&m.content)).sum();
    let sentences: usize = req
        .messages
        .iter()
        .map(|m| {
            m.content
                .split(['.', '!', '?'])
                .filter(|s| !s.trim().is_empty())
                .count()
        })
        .sum();
    let avg_message = total_words / samples;
    let avg_sentence = total_words / sentences.max(1);
    let register = if avg_sentence > 18 {
        "long-form and formal"
    } else {
        "concise and direct"
    };
    format!(
        "Writing style profile (computed on-device):\n\
         - Samples analysed: {samples}\n\
         - Average message length: {avg_message} words\n\
         - Average sentence length: {avg_sentence} words\n\
         - Register: {register}"
    )
}

/// First `max_words` whitespace-normalised words, with a trailing ellipsis
/// when truncated.
#[cfg(not(feature = "local-llm"))]
fn excerpt(text: &str, max_words: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= max_words {
        words.join(" ")
    } else {
        format!("{}…", words[..max_words].join(" "))
    }
}

#[cfg(not(feature = "local-llm"))]
fn last_user_excerpt(req: &ChatRequest, max_words: usize) -> String {
    req.messages
        .iter()
        .rev()
        .find(|m| m.role == ChatRole::User)
        .map(|m| excerpt(&m.content, max_words))
        .unwrap_or_else(|| "your note".into())
}

// ─────────────────────────────────────────────────────────────────────────────
// Real GGUF runtime — compiled only under `--features local-llm`
// ─────────────────────────────────────────────────────────────────────────────

/// In-process GGUF inference through `llama-cpp-2` (card §6 backend choice:
/// GGUF is the community-standard distribution format and the crate ships
/// Metal/CPU backends without extra toolchain complexity; the `ort` generative
/// path stays the reserved alternative).
///
/// > NOTE: because the default delivery build does not enable `local-llm`,
/// > this module is not compiled or unit-tested in CI here; it is written
/// > against the `llama-cpp-2` 0.1.x API and gated so enabling the feature
/// > wires the real runtime. Treat the first `--features local-llm` build as
/// > the integration checkpoint — exactly the stance `embedding/onnx.rs`
/// > (T030) took for `ort`.
#[cfg(feature = "local-llm")]
mod llama {
    use std::num::NonZeroU32;
    use std::path::Path;

    use llama_cpp_2::context::params::LlamaContextParams;
    use llama_cpp_2::llama_backend::LlamaBackend;
    use llama_cpp_2::llama_batch::LlamaBatch;
    use llama_cpp_2::model::params::LlamaModelParams;
    use llama_cpp_2::model::{AddBos, LlamaModel, Special};
    use llama_cpp_2::sampling::LlamaSampler;
    use once_cell::sync::OnceCell;

    use crate::ai::provider::ProviderError;
    use crate::ai::types::ChatRequest;

    use super::GenerativeBackend;

    /// ChatML end-of-turn marker (card §6: ChatML is the default template for
    /// the llama/mistral family).
    const CHATML_END: &str = "<|im_end|>";
    /// Fixed sampler seed so identical requests reproduce identical output.
    const SAMPLER_SEED: u32 = 0x5EEC_E12A;

    /// llama.cpp's global backend may only be initialised once per process;
    /// shared here so unload → reload cycles never re-init it.
    static LLAMA_BACKEND: OnceCell<LlamaBackend> = OnceCell::new();

    fn shared_backend() -> Result<&'static LlamaBackend, ProviderError> {
        LLAMA_BACKEND.get_or_try_init(|| {
            LlamaBackend::init()
                .map_err(|e| ProviderError::Unreachable(format!("llama backend init: {e}")))
        })
    }

    pub struct GgufBackend {
        model: LlamaModel,
        n_ctx: u32,
    }

    impl GgufBackend {
        pub fn load(path: &Path, context_length: usize) -> Result<Self, ProviderError> {
            let backend = shared_backend()?;
            let params = LlamaModelParams::default();
            let model = LlamaModel::load_from_file(backend, path, &params)
                .map_err(|e| ProviderError::Unreachable(format!("load gguf model: {e}")))?;
            Ok(Self {
                model,
                n_ctx: context_length as u32,
            })
        }

        fn format_chatml(req: &ChatRequest) -> String {
            let mut prompt = String::new();
            if !req.system.is_empty() {
                prompt.push_str("<|im_start|>system\n");
                prompt.push_str(&req.system);
                prompt.push_str(CHATML_END);
                prompt.push('\n');
            }
            for message in &req.messages {
                prompt.push_str("<|im_start|>");
                prompt.push_str(message.role.as_str());
                prompt.push('\n');
                prompt.push_str(&message.content);
                prompt.push_str(CHATML_END);
                prompt.push('\n');
            }
            prompt.push_str("<|im_start|>assistant\n");
            prompt
        }
    }

    impl GenerativeBackend for GgufBackend {
        fn generate(&self, req: &ChatRequest) -> Result<String, ProviderError> {
            let backend = shared_backend()?;
            let ctx_params = LlamaContextParams::default().with_n_ctx(NonZeroU32::new(self.n_ctx));
            let mut ctx = self
                .model
                .new_context(backend, ctx_params)
                .map_err(|e| ProviderError::Unreachable(format!("create llama context: {e}")))?;

            let prompt = Self::format_chatml(req);
            let tokens = self
                .model
                .str_to_token(&prompt, AddBos::Always)
                .map_err(|e| ProviderError::BadResponse(format!("tokenize prompt: {e}")))?;
            if tokens.len() >= self.n_ctx as usize {
                return Err(ProviderError::ContextTooLong);
            }

            let mut batch = LlamaBatch::new(self.n_ctx as usize, 1);
            let last = tokens.len() - 1;
            for (i, token) in tokens.iter().enumerate() {
                batch
                    .add(*token, i as i32, &[0], i == last)
                    .map_err(|e| ProviderError::BadResponse(format!("batch add: {e}")))?;
            }
            ctx.decode(&mut batch)
                .map_err(|e| ProviderError::BadResponse(format!("prompt decode: {e}")))?;

            let mut sampler = if req.temperature <= f32::EPSILON {
                LlamaSampler::greedy()
            } else {
                LlamaSampler::chain_simple([
                    LlamaSampler::temp(req.temperature),
                    LlamaSampler::dist(SAMPLER_SEED),
                ])
            };

            let room = self.n_ctx as usize - tokens.len();
            let max_new = (req.max_tokens as usize).min(room);
            let mut out = String::new();
            let mut n_cur = batch.n_tokens();
            for _ in 0..max_new {
                let token = sampler.sample(&ctx, batch.n_tokens() - 1);
                sampler.accept(token);
                if self.model.is_eog_token(token) {
                    break;
                }
                let piece = self
                    .model
                    .token_to_str(token, Special::Tokenize)
                    .map_err(|e| ProviderError::BadResponse(format!("detokenize: {e}")))?;
                out.push_str(&piece);
                if let Some(pos) = out.find(CHATML_END) {
                    out.truncate(pos);
                    break;
                }
                if req.stop.iter().any(|s| !s.is_empty() && out.contains(s)) {
                    break;
                }
                batch.clear();
                batch
                    .add(token, n_cur, &[0], true)
                    .map_err(|e| ProviderError::BadResponse(format!("batch add: {e}")))?;
                n_cur += 1;
                ctx.decode(&mut batch)
                    .map_err(|e| ProviderError::BadResponse(format!("decode: {e}")))?;
            }
            Ok(out)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_model(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"gguf-header-bytes").unwrap();
        path
    }

    #[tokio::test]
    async fn health_without_model_file_is_unreachable() {
        let dir = tempfile::tempdir().unwrap();
        let client = LocalOnnxClient::with_model_dir(dir.path().to_path_buf());
        let err = client.health().await.unwrap_err();
        assert_eq!(
            err,
            ProviderError::Unreachable("model file not found".into())
        );
    }

    #[tokio::test]
    async fn health_with_model_file_reports_ok_and_name() {
        let dir = tempfile::tempdir().unwrap();
        fake_model(dir.path(), "assistant-3b.gguf");
        let client = LocalOnnxClient::with_model_dir(dir.path().to_path_buf());
        let health = client.health().await.unwrap();
        assert!(health.ok);
        assert_eq!(health.model_name.as_deref(), Some("assistant-3b"));
    }

    #[test]
    fn list_local_models_scans_only_generative_files() {
        let dir = tempfile::tempdir().unwrap();
        fake_model(dir.path(), "assistant-3b.gguf");
        std::fs::write(dir.path().join("release-notes.txt"), b"notes").unwrap();
        // Embedding artifacts (T010/T030) must never appear as generative models.
        std::fs::write(dir.path().join("bge-m3.onnx"), b"embedding-model").unwrap();
        std::fs::write(dir.path().join("tokenizer.json"), b"{}").unwrap();

        let models = list_local_generative_models(dir.path());
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].file_name, "assistant-3b.gguf");
        assert_eq!(models[0].size_bytes, "gguf-header-bytes".len() as u64);
    }

    #[test]
    fn missing_directory_lists_no_models() {
        let dir = tempfile::tempdir().unwrap();
        let absent = dir.path().join("never-created");
        assert!(list_local_generative_models(&absent).is_empty());
    }

    #[test]
    fn preferred_model_name_limits_discovery() {
        let dir = tempfile::tempdir().unwrap();
        fake_model(dir.path(), "other-model.gguf");
        assert!(find_model_file_in(dir.path(), Some("wanted.gguf")).is_none());
        assert!(find_model_file_in(dir.path(), Some("other-model.gguf")).is_some());
    }

    #[test]
    fn discovery_prefers_gguf_over_onnx() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("aaa-generative.onnx"), b"onnx").unwrap();
        fake_model(dir.path(), "zzz-assistant.gguf");
        let chosen = find_model_file_in(dir.path(), None).unwrap();
        assert_eq!(
            chosen.file_name().and_then(|n| n.to_str()),
            Some("zzz-assistant.gguf")
        );
    }

    #[test]
    fn stream_chunks_concatenate_to_original_text() {
        let text = "one two  three\nfour five six seven eight nine ten eleven";
        let chunks = chunk_for_stream(text);
        assert!(chunks.len() > 1);
        assert_eq!(chunks.concat(), text);
        assert!(chunk_for_stream("").is_empty());
    }

    // Behavior of the deterministic offline backend (default build only — a
    // `local-llm` build runs real inference and is integration-tested with an
    // actual model instead, mirroring `embedding/onnx.rs`).
    #[cfg(not(feature = "local-llm"))]
    mod offline {
        use std::sync::Arc;

        use futures::StreamExt;

        use super::super::*;
        use super::fake_model;
        use crate::ai::types::Capability;

        fn ready_client(dir: &Path) -> LocalOnnxClient {
            fake_model(dir, "assistant-3b.gguf");
            LocalOnnxClient::with_model_dir(dir.to_path_buf())
        }

        fn summarize_request() -> ChatRequest {
            ChatRequest::simple(
                "assistant-3b",
                "The vendor invoice for the March shipment needs sign-off before Friday.",
                Capability::Summarize,
            )
        }

        #[tokio::test]
        async fn chat_without_model_file_is_unreachable() {
            let dir = tempfile::tempdir().unwrap();
            let client = LocalOnnxClient::with_model_dir(dir.path().to_path_buf());
            let err = client.chat(summarize_request()).await.unwrap_err();
            assert_eq!(
                err,
                ProviderError::Unreachable("local model not installed".into())
            );
        }

        #[tokio::test]
        async fn lazy_load_initialises_once_under_concurrent_chats() {
            let dir = tempfile::tempdir().unwrap();
            let client = Arc::new(ready_client(dir.path()));

            let mut handles = Vec::new();
            for _ in 0..4 {
                let client = client.clone();
                handles.push(tokio::spawn(async move {
                    client.chat(summarize_request()).await
                }));
            }
            for handle in handles {
                assert!(handle.await.unwrap().is_ok());
            }
            assert_eq!(client.slot.loads.load(Ordering::SeqCst), 1);
        }

        #[tokio::test]
        async fn offline_completion_is_deterministic() {
            let dir = tempfile::tempdir().unwrap();
            let client = ready_client(dir.path());
            let first = client.chat(summarize_request()).await.unwrap();
            let second = client.chat(summarize_request()).await.unwrap();
            assert_eq!(first.text, second.text);
            assert_eq!(first.finish, FinishReason::Stop);
            assert_eq!(first.model_echo, "assistant-3b");
            assert!(first.usage.completion_tokens > 0);
        }

        #[tokio::test]
        async fn each_capability_produces_structured_text() {
            let dir = tempfile::tempdir().unwrap();
            let client = ready_client(dir.path());
            for purpose in [
                Capability::Summarize,
                Capability::DraftReply,
                Capability::RiskReason,
                Capability::StyleProfile,
            ] {
                let req = ChatRequest::simple(
                    "assistant-3b",
                    "Please confirm the settlement schedule with the counterparty.",
                    purpose,
                );
                let resp = client.chat(req).await.unwrap();
                assert!(
                    !resp.text.trim().is_empty(),
                    "capability {} produced empty text",
                    purpose.as_str()
                );
            }
        }

        #[tokio::test]
        async fn stream_deltas_reassemble_to_chat_text() {
            let dir = tempfile::tempdir().unwrap();
            let client = ready_client(dir.path());
            let full = client.chat(summarize_request()).await.unwrap();

            let mut stream = client.chat_stream(summarize_request()).await.unwrap();
            let mut collected = String::new();
            let mut expected_index = 0usize;
            while let Some(delta) = stream.next().await {
                let delta = delta.unwrap();
                assert_eq!(delta.index, expected_index);
                expected_index += 1;
                collected.push_str(&delta.text);
            }
            assert!(expected_index > 1, "expected multiple deltas");
            assert_eq!(collected, full.text);
        }

        #[tokio::test]
        async fn stop_sequence_truncates_completion() {
            let dir = tempfile::tempdir().unwrap();
            let client = ready_client(dir.path());

            let unstopped = client.chat(summarize_request()).await.unwrap();
            assert!(unstopped.text.contains("Action:"));

            let mut req = summarize_request();
            req.stop = vec!["Action:".into()];
            let stopped = client.chat(req).await.unwrap();
            assert!(!stopped.text.contains("Action:"));
            assert_eq!(stopped.finish, FinishReason::Stop);
        }

        #[tokio::test]
        async fn max_tokens_budget_truncates_with_length_finish() {
            let dir = tempfile::tempdir().unwrap();
            let client = ready_client(dir.path());
            let mut req = summarize_request();
            req.max_tokens = 5;
            let resp = client.chat(req).await.unwrap();
            assert!(resp.text.split_whitespace().count() <= 5);
            assert_eq!(resp.finish, FinishReason::Length);
        }

        #[tokio::test]
        async fn oversized_prompt_is_rejected_before_inference() {
            let dir = tempfile::tempdir().unwrap();
            let client = ready_client(dir.path());
            let huge = "word ".repeat(DEFAULT_CONTEXT_WINDOW + 1);
            let req = ChatRequest::simple("assistant-3b", huge, Capability::Summarize);
            let err = client.chat(req).await.unwrap_err();
            assert_eq!(err, ProviderError::ContextTooLong);
            // Rejected before any load happened.
            assert_eq!(client.slot.loads.load(Ordering::SeqCst), 0);
        }

        #[tokio::test]
        async fn idle_unload_drops_model_and_next_call_reloads() {
            let dir = tempfile::tempdir().unwrap();
            let client = ready_client(dir.path());
            client.chat(summarize_request()).await.unwrap();
            assert_eq!(client.slot.loads.load(Ordering::SeqCst), 1);

            let limit = Duration::from_secs(IDLE_UNLOAD_SECS);
            // Recently used → the watchdog leaves the model in memory.
            assert!(!client.slot.unload_if_idle(Instant::now(), limit).await);
            // Injected future clock → idle limit exceeded → unload.
            assert!(
                client
                    .slot
                    .unload_if_idle(Instant::now() + limit, limit)
                    .await
            );
            // Next call lazily reloads.
            client.chat(summarize_request()).await.unwrap();
            assert_eq!(client.slot.loads.load(Ordering::SeqCst), 2);
        }

        #[tokio::test]
        async fn from_config_builds_registrable_client() {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().to_path_buf();
            let paths = Paths {
                db: root.join("seekermail.db"),
                vectors: root.join("vectors"),
                attachments: root.join("attachments"),
                logs: root.join("logs"),
                models: root.join("models"),
                root,
            };
            std::fs::create_dir_all(&paths.models).unwrap();
            fake_model(&paths.models, "assistant-3b.gguf");

            let cfg = AccountAiConfig {
                account_id: "acct-1".into(),
                provider: AiProvider::LocalOnnx,
                model: Some("assistant-3b.gguf".into()),
                base_url: None,
                api_key_ref: None,
                daily_query_limit: 100,
                updated_at: 0,
            };
            let client = LocalOnnxClient::from_config(&cfg, &paths).unwrap();
            assert_eq!(client.id(), AiProvider::LocalOnnx);
            assert_eq!(client.context_window(), DEFAULT_CONTEXT_WINDOW);
            assert!(client.health().await.unwrap().ok);

            // A configured name that is not on disk → not installed.
            let missing_cfg = AccountAiConfig {
                model: Some("not-downloaded.gguf".into()),
                ..cfg
            };
            let client = LocalOnnxClient::from_config(&missing_cfg, &paths).unwrap();
            assert!(client.health().await.is_err());
        }
    }
}
