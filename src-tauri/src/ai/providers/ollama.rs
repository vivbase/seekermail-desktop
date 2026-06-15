//! Local Ollama adapter (T062, dev/06 §1, F_F2).
//!
//! Maps [`AiProviderClient`] onto a local Ollama daemon through its
//! OpenAI-compatible route (`{base}/v1/chat/completions`, default
//! `http://localhost:11434`). Ollama is a **local** provider: no API key, no
//! Keychain access, no data-flow disclosure — mail content never leaves the
//! device (dev/06 §1, §8; ADR-0004).
//!
//! Local-provider specifics (dev/06 §6, F_F2 §4):
//!
//! * Timeouts: connect 10 s (loopback answers instantly), total **120 s** —
//!   the first inference after daemon start can spend 30+ s loading weights.
//! * Concurrency: a `tokio::sync::Semaphore` (default 1 permit, max 4) keeps
//!   the daemon from juggling parallel inferences and degrading quality.
//! * Discovery: [`discover_ollama_models`] reads `/api/tags` for the config
//!   UI (T068); [`scan_default_ollama_endpoints`] probes the default ports.
//!
//! Log safety (dev/09 §5): nothing in this file ever places prompt text,
//! completion text, or response bodies into error payloads or log lines —
//! only endpoint kind, HTTP status, and delta ordinals.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::stream::{self, BoxStream};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::ai::provider::{AiProviderClient, ChatDeltaStream, ProviderError};
use crate::ai::registry::AccountAiConfig;
use crate::ai::types::{
    ChatDelta, ChatRequest, ChatResponse, FinishReason, ProviderHealth, TokenUsage,
};
use crate::error::{AppError, AppResult};
use crate::types::AiProvider;

/// Default daemon endpoint (dev/06 §1, F_F2 §4.1).
pub const DEFAULT_BASE_URL: &str = "http://localhost:11434";

/// Endpoints probed by [`scan_default_ollama_endpoints`] (F_F2 §3).
pub const DEFAULT_OLLAMA_ENDPOINTS: [&str; 2] =
    ["http://localhost:11434", "http://127.0.0.1:11434"];

/// Loopback connections answer instantly — no reason to wait longer (dev/06 §6).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Local models can take 30+ s to load into memory on first call (dev/06 §6).
const TOTAL_TIMEOUT: Duration = Duration::from_secs(120);
/// Endpoint scanning must feel instant in the config UI (F_F2 §3).
const SCAN_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const SCAN_TOTAL_TIMEOUT: Duration = Duration::from_secs(3);
/// Model discovery is a metadata read, not an inference — keep it snappy.
const DISCOVERY_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const DISCOVERY_TOTAL_TIMEOUT: Duration = Duration::from_secs(10);

/// Conservative window when the model name gives no hint (T062 card §3).
const FALLBACK_CONTEXT_WINDOW: usize = 4_096;
/// One inference at a time by default; hard cap 4 (F_F2 §4.4).
const DEFAULT_MAX_CONCURRENCY: usize = 1;
const MAX_CONCURRENCY_CAP: usize = 4;

/// One installed model as reported by `GET /api/tags` (F_F2 §4.3). Consumed by
/// the T068 provider-config wizard through the `list_ollama_models` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OllamaModelInfo {
    /// Full model tag, e.g. `llama3:8b`.
    pub name: String,
    /// On-disk weight size in bytes (0 when the daemon omits it).
    pub size_bytes: u64,
    /// e.g. `8B` — from `details.parameter_size`.
    pub parameter_size: Option<String>,
    /// e.g. `Q4_0` — from `details.quantization_level`.
    pub quantization: Option<String>,
}

/// `AiProviderClient` adapter for a local Ollama daemon.
pub struct OllamaClient {
    model: String,
    base_url: String,
    context_window: usize,
    http: reqwest::Client,
    /// Serializes inferences toward the daemon (F_F2 §4.4).
    semaphore: Arc<Semaphore>,
}

impl std::fmt::Debug for OllamaClient {
    /// Identifier-only `Debug`: never prints endpoints-with-keys, key
    /// sources, or any request/response content (09 Â§5).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OllamaClient")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl OllamaClient {
    /// Build a client for one daemon + model. No `api_key_ref` parameter by
    /// design — local providers hold no credentials and never touch the
    /// Keychain (T062 card §6).
    ///
    /// `context_length` overrides the model-name heuristic when the caller
    /// knows the real window; `max_concurrency` is clamped to `1..=4`.
    pub fn new(
        model: impl Into<String>,
        base_url: Option<String>,
        context_length: Option<usize>,
        max_concurrency: usize,
    ) -> AppResult<Self> {
        let model = model.into();
        let base_url = normalize_base_url(base_url.as_deref().unwrap_or(DEFAULT_BASE_URL));
        let context_window = context_length
            .or_else(|| heuristic_context_window(&model))
            .unwrap_or(FALLBACK_CONTEXT_WINDOW);
        let http = build_client(CONNECT_TIMEOUT, TOTAL_TIMEOUT)
            .map_err(|detail| AppError::Internal(anyhow::anyhow!("ollama {detail}")))?;
        let permits = max_concurrency.clamp(1, MAX_CONCURRENCY_CAP);
        Ok(Self {
            model,
            base_url,
            context_window,
            http,
            semaphore: Arc::new(Semaphore::new(permits)),
        })
    }

    /// Registry factory: build the per-account adapter from
    /// `account_ai_settings` (the model and base URL are account-level
    /// choices, so Ollama registers as a factory, not a singleton).
    pub fn from_config(cfg: &AccountAiConfig) -> AppResult<Arc<Self>> {
        let model = cfg
            .model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .ok_or_else(|| AppError::Validation("ollama provider requires a model name".into()))?;
        let client = Self::new(model, cfg.base_url.clone(), None, DEFAULT_MAX_CONCURRENCY)?;
        Ok(Arc::new(client))
    }

    /// One-shot reachability probe backing `verify_ai_provider` (dev/06 §8).
    /// Cross-adapter convention shares this signature; `_api_key` is accepted
    /// and ignored — Ollama is a local provider with no credentials.
    pub async fn probe(
        model: &str,
        _api_key: Option<&str>,
        base_url: Option<&str>,
    ) -> Result<ProviderHealth, ProviderError> {
        let client = Self::new(model, base_url.map(str::to_string), None, 1)
            .map_err(|e| ProviderError::Unreachable(format!("client init failed: {e}")))?;
        client.health().await
    }

    /// Local providers never send mail content off-device (dev/06 §1). The
    /// T069 data-flow panel keys off `AiProvider::Ollama`; this accessor is
    /// the adapter-level statement of the same fact.
    pub fn is_local(&self) -> bool {
        true
    }

    fn chat_url(&self) -> String {
        format!("{}/v1/chat/completions", self.base_url)
    }

    fn tags_url(&self) -> String {
        format!("{}/api/tags", self.base_url)
    }
}

#[async_trait]
impl AiProviderClient for OllamaClient {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let _permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| ProviderError::Canceled)?;

        tracing::debug!(
            event = "ollama_chat_request",
            model = %req.model,
            request_id = %req.request_id,
            purpose = req.purpose.as_str(),
            "sending non-streaming completion to local daemon"
        );

        let wire_req = build_wire_request(&req, false);
        let started = Instant::now();
        let resp = self
            .http
            .post(self.chat_url())
            .json(&wire_req)
            .send()
            .await
            .map_err(map_transport_error)?;
        let resp = ensure_success(resp).await?;
        let wire: WireChatResponse = resp
            .json()
            .await
            .map_err(|_| ProviderError::BadResponse("unparseable chat completion body".into()))?;
        let latency_ms = elapsed_ms(started);

        let choice = wire.choices.into_iter().next().ok_or_else(|| {
            ProviderError::BadResponse("chat completion contained no choices".into())
        })?;
        let usage = wire
            .usage
            .map(|u| TokenUsage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
            })
            .unwrap_or_default();

        Ok(ChatResponse {
            text: choice.message.content,
            finish: map_finish_reason(choice.finish_reason.as_deref()),
            usage,
            model_echo: if wire.model.is_empty() {
                req.model
            } else {
                wire.model
            },
            latency_ms,
        })
    }

    async fn chat_stream(&self, req: ChatRequest) -> Result<ChatDeltaStream, ProviderError> {
        // The permit rides inside the stream state so a long generation keeps
        // the daemon exclusive until the stream finishes or is dropped.
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| ProviderError::Canceled)?;

        tracing::debug!(
            event = "ollama_chat_stream_request",
            model = %req.model,
            request_id = %req.request_id,
            purpose = req.purpose.as_str(),
            "opening streaming completion to local daemon"
        );

        let wire_req = build_wire_request(&req, true);
        let resp = self
            .http
            .post(self.chat_url())
            .json(&wire_req)
            .send()
            .await
            .map_err(map_transport_error)?;
        let resp = ensure_success(resp).await?;

        let body = resp
            .bytes_stream()
            .map(|chunk| chunk.map(|b| b.to_vec()))
            .boxed();
        Ok(delta_stream(body, permit))
    }

    async fn health(&self) -> Result<ProviderHealth, ProviderError> {
        let started = Instant::now();
        let resp = self
            .http
            .get(self.tags_url())
            .send()
            .await
            .map_err(map_transport_error)?;
        // Headers received ≈ first byte: this is the TTFB shown by the T068
        // provider panel (F_F2 §4.2).
        let latency_ms = elapsed_ms(started);
        let resp = ensure_success(resp).await?;
        let tags: WireTagsResponse = resp
            .json()
            .await
            .map_err(|_| ProviderError::BadResponse("unparseable /api/tags body".into()))?;

        // `/api/tags` lists installed models; echo the configured one back
        // when the daemon actually has it.
        let model_name = tags
            .models
            .iter()
            .find(|m| model_matches(&m.name, &self.model))
            .map(|m| m.name.clone());

        Ok(ProviderHealth {
            ok: true,
            model_name,
            latency_ms,
        })
    }

    fn id(&self) -> AiProvider {
        AiProvider::Ollama
    }

    fn context_window(&self) -> usize {
        self.context_window
    }
}

// ── Model discovery & endpoint scanning (F_F2 §3, T068 consumers) ───────────

/// List the models installed on a daemon via `GET /api/tags`. `None` probes
/// the default endpoint. Backs the `list_ollama_models` command (T068).
pub async fn discover_ollama_models(
    base_url: Option<&str>,
) -> Result<Vec<OllamaModelInfo>, ProviderError> {
    let base = normalize_base_url(base_url.unwrap_or(DEFAULT_BASE_URL));
    let client = build_client(DISCOVERY_CONNECT_TIMEOUT, DISCOVERY_TOTAL_TIMEOUT)
        .map_err(ProviderError::Unreachable)?;
    let resp = client
        .get(format!("{base}/api/tags"))
        .send()
        .await
        .map_err(map_transport_error)?;
    let resp = ensure_success(resp).await?;
    let tags: WireTagsResponse = resp
        .json()
        .await
        .map_err(|_| ProviderError::BadResponse("unparseable /api/tags body".into()))?;

    Ok(tags
        .models
        .into_iter()
        .map(|m| OllamaModelInfo {
            name: m.name,
            size_bytes: m.size.unwrap_or(0),
            parameter_size: m.details.as_ref().and_then(|d| d.parameter_size.clone()),
            quantization: m.details.and_then(|d| d.quantization_level),
        })
        .collect())
}

/// Probe the default local endpoints and return the reachable ones, in probe
/// order. Backs the `scan_local_providers` command (T068, F_F2 §3).
pub async fn scan_default_ollama_endpoints() -> Vec<String> {
    scan_endpoints(&DEFAULT_OLLAMA_ENDPOINTS).await
}

/// Sequentially probe `GET {base}/api/tags` on each candidate with a 2 s
/// connect timeout; collect the bases that answer 2xx.
async fn scan_endpoints(candidates: &[&str]) -> Vec<String> {
    let Ok(client) = build_client(SCAN_CONNECT_TIMEOUT, SCAN_TOTAL_TIMEOUT) else {
        return Vec::new();
    };
    let mut reachable = Vec::new();
    for candidate in candidates {
        let base = normalize_base_url(candidate);
        let url = format!("{base}/api/tags");
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => reachable.push(base),
            _ => {}
        }
    }
    reachable
}

// ── Wire shapes (OpenAI-compatible route) ────────────────────────────────────

#[derive(Serialize)]
struct WireMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct WireChatRequest<'a> {
    model: &'a str,
    messages: Vec<WireMessage<'a>>,
    max_tokens: u32,
    temperature: f32,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    stop: &'a [String],
    stream: bool,
}

#[derive(Deserialize)]
struct WireChatResponse {
    #[serde(default)]
    model: String,
    #[serde(default)]
    choices: Vec<WireChoice>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct WireChoice {
    message: WireResponseMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct WireResponseMessage {
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

#[derive(Deserialize)]
struct WireStreamChunk {
    #[serde(default)]
    choices: Vec<WireStreamChoice>,
}

#[derive(Deserialize)]
struct WireStreamChoice {
    #[serde(default)]
    delta: WireStreamDelta,
}

#[derive(Deserialize, Default)]
struct WireStreamDelta {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
struct WireTagsResponse {
    #[serde(default)]
    models: Vec<WireTagModel>,
}

#[derive(Deserialize)]
struct WireTagModel {
    name: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    details: Option<WireTagDetails>,
}

#[derive(Deserialize)]
struct WireTagDetails {
    #[serde(default)]
    parameter_size: Option<String>,
    #[serde(default)]
    quantization_level: Option<String>,
}

/// Map the neutral request onto the OpenAI-compatible body. The system
/// preamble travels as a leading `system` message on this route.
fn build_wire_request(req: &ChatRequest, stream: bool) -> WireChatRequest<'_> {
    let mut messages = Vec::with_capacity(req.messages.len() + 1);
    if !req.system.is_empty() {
        messages.push(WireMessage {
            role: "system",
            content: &req.system,
        });
    }
    for m in &req.messages {
        messages.push(WireMessage {
            role: m.role.as_str(),
            content: &m.content,
        });
    }
    WireChatRequest {
        model: &req.model,
        messages,
        max_tokens: req.max_tokens,
        temperature: req.temperature,
        stop: &req.stop,
        stream,
    }
}

// ── SSE delta stream (private to this adapter; T061 may lift it later) ──────

/// Reassembles SSE lines from arbitrary byte-chunk boundaries. The daemon may
/// split a `data:` line across TCP reads, so complete lines are only released
/// once their terminating `\n` arrives (`\r\n` tolerated).
#[derive(Default)]
struct SseLineBuffer {
    buf: Vec<u8>,
}

impl SseLineBuffer {
    /// Feed one raw chunk; get back every line completed by it.
    fn push_chunk(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(chunk);
        let mut lines = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop(); // trailing \n
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            lines.push(String::from_utf8_lossy(&line).into_owned());
        }
        lines
    }

    /// Release a final unterminated line at end-of-body, if any.
    fn flush(&mut self) -> Option<String> {
        if self.buf.is_empty() {
            return None;
        }
        let line = String::from_utf8_lossy(&self.buf).into_owned();
        self.buf.clear();
        Some(line)
    }
}

enum SseLine {
    /// A `data:` line carrying a JSON chunk payload.
    Data(String),
    /// The `data: [DONE]` terminator.
    Done,
    /// Empty lines, comments, `event:` fields — anything non-data.
    Ignore,
}

fn classify_sse_line(line: &str) -> SseLine {
    let trimmed = line.trim();
    let Some(payload) = trimmed.strip_prefix("data:") else {
        return SseLine::Ignore;
    };
    let payload = payload.trim();
    if payload.is_empty() {
        SseLine::Ignore
    } else if payload == "[DONE]" {
        SseLine::Done
    } else {
        SseLine::Data(payload.to_string())
    }
}

/// Pull the delta text out of one `data:` JSON payload. Empty/role-only
/// chunks yield `Ok(None)`. The error payload is a fixed string — the raw
/// payload must never travel inside a `ProviderError` (dev/09 §5).
fn extract_delta_text(payload: &str) -> Result<Option<String>, ProviderError> {
    let chunk: WireStreamChunk = serde_json::from_str(payload)
        .map_err(|_| ProviderError::BadResponse("unparseable sse delta chunk".into()))?;
    Ok(chunk
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.delta.content)
        .filter(|text| !text.is_empty()))
}

struct StreamState {
    body: BoxStream<'static, Result<Vec<u8>, reqwest::Error>>,
    lines: SseLineBuffer,
    pending: VecDeque<ChatDelta>,
    error: Option<ProviderError>,
    next_index: usize,
    finished: bool,
    /// Holds the concurrency slot until the stream ends or is dropped.
    _permit: OwnedSemaphorePermit,
}

impl StreamState {
    fn consume_line(&mut self, line: &str) {
        if self.finished || self.error.is_some() {
            return;
        }
        match classify_sse_line(line) {
            SseLine::Done => self.finished = true,
            SseLine::Ignore => {}
            SseLine::Data(payload) => match extract_delta_text(&payload) {
                Ok(Some(text)) => {
                    self.pending.push_back(ChatDelta {
                        text,
                        index: self.next_index,
                    });
                    self.next_index += 1;
                }
                Ok(None) => {}
                Err(e) => self.error = Some(e),
            },
        }
    }
}

/// Adapt the raw response body into a `ChatDelta` stream via
/// `futures::stream::unfold` (no hand-rolled `Stream` impl).
fn delta_stream(
    body: BoxStream<'static, Result<Vec<u8>, reqwest::Error>>,
    permit: OwnedSemaphorePermit,
) -> ChatDeltaStream {
    let state = StreamState {
        body,
        lines: SseLineBuffer::default(),
        pending: VecDeque::new(),
        error: None,
        next_index: 0,
        finished: false,
        _permit: permit,
    };

    stream::unfold(state, |mut st| async move {
        loop {
            // Drain parsed deltas first, then a deferred error, then end.
            if let Some(delta) = st.pending.pop_front() {
                return Some((Ok(delta), st));
            }
            if let Some(err) = st.error.take() {
                st.finished = true;
                return Some((Err(err), st));
            }
            if st.finished {
                return None;
            }
            match st.body.next().await {
                Some(Ok(chunk)) => {
                    for line in st.lines.push_chunk(&chunk) {
                        st.consume_line(&line);
                    }
                }
                Some(Err(e)) => {
                    st.error = Some(map_transport_error(e));
                }
                None => {
                    // End of body without `[DONE]`: parse any unterminated
                    // tail, then finish gracefully.
                    if let Some(line) = st.lines.flush() {
                        st.consume_line(&line);
                    }
                    st.finished = true;
                }
            }
        }
    })
    .boxed()
}

// ── Shared HTTP plumbing ─────────────────────────────────────────────────────

/// Build the HTTP client. `no_proxy` is deliberate: traffic to the local
/// daemon must never detour through an environment-configured proxy
/// (ADR-0004 — direct connection, nothing in the path).
fn build_client(connect: Duration, total: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(connect)
        .timeout(total)
        .no_proxy()
        .build()
        .map_err(|e| format!("http client init failed: {e}"))
}

/// Map a non-2xx status to the provider error model (dev/06 §6). Payloads
/// carry the status only — never the response body, which could echo content.
async fn ensure_success(resp: reqwest::Response) -> Result<reqwest::Response, ProviderError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    match status.as_u16() {
        401 | 403 => Err(ProviderError::Auth),
        429 => {
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            Err(ProviderError::RateLimited { retry_after })
        }
        400 => {
            // Inspect (but never propagate) the body to spot the context
            // overflow class of 400s.
            let body = resp.text().await.unwrap_or_default().to_ascii_lowercase();
            if body.contains("context length")
                || body.contains("context_length")
                || body.contains("maximum context")
                || body.contains("too long")
            {
                Err(ProviderError::ContextTooLong)
            } else {
                Err(ProviderError::BadResponse(
                    "http 400 from ollama chat endpoint".into(),
                ))
            }
        }
        404 => Err(ProviderError::BadResponse(
            "http 404 from ollama endpoint (model not installed on daemon?)".into(),
        )),
        s => Err(ProviderError::BadResponse(format!(
            "http {s} from ollama endpoint"
        ))),
    }
}

/// Map transport-level failures (dev/06 §6, T062 card §6): refused/timeout →
/// `Unreachable` (daemon not running or still loading), decode → `BadResponse`.
fn map_transport_error(e: reqwest::Error) -> ProviderError {
    if e.is_timeout() {
        ProviderError::Unreachable("request to local daemon timed out".into())
    } else if e.is_connect() {
        ProviderError::Unreachable("connection to local daemon refused or failed".into())
    } else if e.is_decode() {
        ProviderError::BadResponse("response body decode error".into())
    } else {
        ProviderError::Unreachable("network transport error toward local daemon".into())
    }
}

fn map_finish_reason(reason: Option<&str>) -> FinishReason {
    match reason {
        None | Some("stop") => FinishReason::Stop,
        Some("length") => FinishReason::Length,
        Some("content_filter") => FinishReason::ContentFilter,
        Some(_) => FinishReason::Error,
    }
}

fn elapsed_ms(started: Instant) -> u32 {
    u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX)
}

fn normalize_base_url(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        DEFAULT_BASE_URL.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Does an installed tag satisfy the configured model name? Exact match, or
/// the configured name matches the tag's family (`llama3` ↔ `llama3:latest`).
fn model_matches(installed: &str, configured: &str) -> bool {
    installed == configured || installed.split(':').next() == Some(configured)
}

/// Conservative context-window guess from the model tag (T062 card §3). Used
/// only when the caller did not inject a known `context_length`; unknown
/// families fall back to [`FALLBACK_CONTEXT_WINDOW`].
fn heuristic_context_window(model: &str) -> Option<usize> {
    let m = model.to_ascii_lowercase();
    if m.contains("llama3.1") || m.contains("llama3.2") || m.contains("llama3.3") {
        Some(131_072)
    } else if m.contains("llama3") || m.contains("llama-3") {
        Some(8_192)
    } else if m.contains("llama2") || m.contains("llama-2") {
        Some(4_096)
    } else if m.contains("qwen2") || m.contains("mixtral") || m.contains("mistral") {
        Some(32_768)
    } else if m.contains("gemma") {
        Some(8_192)
    } else if m.contains("phi3") || m.contains("phi-3") {
        Some(4_096)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::types::Capability;
    use std::sync::Mutex;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Respond, ResponseTemplate};

    fn chat_completion_json(text: &str) -> serde_json::Value {
        serde_json::json!({
            "id": "chatcmpl-417",
            "object": "chat.completion",
            "model": "llama3:8b",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": text },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 42, "completion_tokens": 7, "total_tokens": 49 }
        })
    }

    fn tags_json() -> serde_json::Value {
        serde_json::json!({
            "models": [
                {
                    "name": "llama3:8b",
                    "size": 4_661_224_676u64,
                    "details": { "parameter_size": "8B", "quantization_level": "Q4_0" }
                },
                {
                    "name": "qwen2.5:14b",
                    "size": 8_988_124_069u64,
                    "details": { "parameter_size": "14B", "quantization_level": "Q4_K_M" }
                }
            ]
        })
    }

    fn client_for(base: &str) -> OllamaClient {
        OllamaClient::new("llama3:8b", Some(base.to_string()), None, 1).unwrap()
    }

    fn simple_request() -> ChatRequest {
        ChatRequest::simple(
            "llama3:8b",
            "summarize the latest thread",
            Capability::Summarize,
        )
    }

    /// A localhost base URL where nothing listens (bind, read port, drop).
    fn refused_base_url() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        format!("http://127.0.0.1:{port}")
    }

    // ── chat ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ollama_chat_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(chat_completion_json("Summary ready.")),
            )
            .mount(&server)
            .await;

        let client = client_for(&server.uri());
        let resp = client.chat(simple_request()).await.unwrap();

        assert_eq!(resp.text, "Summary ready.");
        assert_eq!(resp.finish, FinishReason::Stop);
        assert_eq!(resp.usage.prompt_tokens, 42);
        assert_eq!(resp.usage.completion_tokens, 7);
        assert_eq!(resp.model_echo, "llama3:8b");

        // Local provider: no Authorization header, stream disabled, the user
        // turn on the wire.
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].headers.get("authorization").is_none());
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["model"], "llama3:8b");
        assert_eq!(body["stream"], false);
        assert_eq!(body["messages"][0]["role"], "user");
    }

    #[tokio::test]
    async fn ollama_chat_unreachable_on_connect_refused() {
        let client = client_for(&refused_base_url());
        let err = client.chat(simple_request()).await.unwrap_err();
        assert!(matches!(err, ProviderError::Unreachable(_)));
    }

    #[tokio::test]
    async fn ollama_chat_404_maps_to_bad_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(404).set_body_json(
                serde_json::json!({ "error": { "message": "model 'llama3:8b' not found" } }),
            ))
            .mount(&server)
            .await;

        let err = client_for(&server.uri())
            .chat(simple_request())
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::BadResponse(_)));
    }

    #[tokio::test]
    async fn ollama_chat_400_context_class_maps_to_context_too_long() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(400).set_body_json(
                serde_json::json!({ "error": { "message": "this model's maximum context length is exceeded" } }),
            ))
            .mount(&server)
            .await;

        let err = client_for(&server.uri())
            .chat(simple_request())
            .await
            .unwrap_err();
        assert_eq!(err, ProviderError::ContextTooLong);
    }

    /// dev/09 §5: a 5xx body that echoes content must never surface in the
    /// error payload (only the status code does).
    #[tokio::test]
    async fn ollama_error_payload_never_carries_body_content() {
        const SENSITIVE: &str = "the settlement amount is $4,200,000";
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string(SENSITIVE))
            .mount(&server)
            .await;

        let err = client_for(&server.uri())
            .chat(simple_request())
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::BadResponse(_)));
        assert!(!format!("{err}").contains("settlement"));
    }

    // ── chat_stream ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ollama_chat_stream_deltas() {
        let sse = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hel\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo \"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"there\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(sse.as_bytes().to_vec(), "text/event-stream"),
            )
            .mount(&server)
            .await;

        let client = client_for(&server.uri());
        let mut stream = client.chat_stream(simple_request()).await.unwrap();
        let mut deltas = Vec::new();
        while let Some(item) = stream.next().await {
            deltas.push(item.unwrap());
        }

        assert_eq!(deltas.len(), 3);
        assert_eq!(
            deltas[0],
            ChatDelta {
                text: "Hel".into(),
                index: 0
            }
        );
        assert_eq!(
            deltas[1],
            ChatDelta {
                text: "lo ".into(),
                index: 1
            }
        );
        assert_eq!(
            deltas[2],
            ChatDelta {
                text: "there".into(),
                index: 2
            }
        );
    }

    /// Raw byte chunks split mid-line and mid-`data:` prefix must reassemble
    /// into the same delta sequence.
    #[test]
    fn sse_parser_handles_chunks_split_mid_line() {
        let chunks: [&[u8]; 4] = [
            b"data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\nda",
            b"ta: {\"choices\":[{\"delta\":{\"content\":\"lo wo\"}}]}\r\ndata: {\"choices\":[{\"delta\":{\"con",
            b"tent\":\"rld\"}}]}\n",
            b"data: [DONE]\n",
        ];

        let mut buf = SseLineBuffer::default();
        let mut text = String::new();
        let mut done = false;
        for chunk in chunks {
            for line in buf.push_chunk(chunk) {
                match classify_sse_line(&line) {
                    SseLine::Data(payload) => {
                        if let Some(t) = extract_delta_text(&payload).unwrap() {
                            text.push_str(&t);
                        }
                    }
                    SseLine::Done => done = true,
                    SseLine::Ignore => {}
                }
            }
        }

        assert_eq!(text, "Hello world");
        assert!(done);
    }

    #[test]
    fn sse_buffer_flushes_trailing_line_without_newline() {
        let mut buf = SseLineBuffer::default();
        assert!(buf.push_chunk(b"data: [DONE]").is_empty());
        let line = buf.flush().unwrap();
        assert!(matches!(classify_sse_line(&line), SseLine::Done));
        assert!(buf.flush().is_none());
    }

    #[test]
    fn sse_parser_rejects_malformed_chunk_without_echoing_it() {
        let err = extract_delta_text("{not json, contains private words}").unwrap_err();
        assert!(matches!(err, ProviderError::BadResponse(_)));
        assert!(!format!("{err}").contains("private words"));
    }

    // ── health / probe ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn ollama_health_ok_reports_installed_model() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(tags_json()))
            .mount(&server)
            .await;

        let health = client_for(&server.uri()).health().await.unwrap();
        assert!(health.ok);
        assert_eq!(health.model_name.as_deref(), Some("llama3:8b"));
    }

    #[tokio::test]
    async fn ollama_health_unreachable() {
        let err = client_for(&refused_base_url()).health().await.unwrap_err();
        assert!(matches!(err, ProviderError::Unreachable(_)));
    }

    #[tokio::test]
    async fn ollama_probe_ignores_api_key() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(tags_json()))
            .mount(&server)
            .await;

        let uri = server.uri();
        let health = OllamaClient::probe("llama3:8b", Some("ignored-key"), Some(&uri))
            .await
            .unwrap();
        assert!(health.ok);
        // No credential header may ever reach the daemon.
        let requests = server.received_requests().await.unwrap();
        assert!(requests
            .iter()
            .all(|r| r.headers.get("authorization").is_none()));
    }

    // ── discovery & scanning ────────────────────────────────────────────────

    #[tokio::test]
    async fn ollama_discover_models_parses_tags() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(tags_json()))
            .mount(&server)
            .await;

        let uri = server.uri();
        let models = discover_ollama_models(Some(&uri)).await.unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].name, "llama3:8b");
        assert_eq!(models[0].size_bytes, 4_661_224_676);
        assert_eq!(models[0].parameter_size.as_deref(), Some("8B"));
        assert_eq!(models[0].quantization.as_deref(), Some("Q4_0"));
        assert_eq!(models[1].name, "qwen2.5:14b");
        assert_eq!(models[1].quantization.as_deref(), Some("Q4_K_M"));
    }

    #[tokio::test]
    async fn ollama_scan_finds_reachable() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "models": [] })),
            )
            .mount(&server)
            .await;

        let live = server.uri();
        let dead = refused_base_url();
        let found = scan_endpoints(&[live.as_str(), dead.as_str()]).await;
        assert_eq!(found, vec![live]);
    }

    // ── concurrency guard ───────────────────────────────────────────────────

    struct TrackingResponder {
        arrivals: Arc<Mutex<Vec<Instant>>>,
        delay: Duration,
    }

    impl Respond for TrackingResponder {
        fn respond(&self, _request: &wiremock::Request) -> ResponseTemplate {
            self.arrivals.lock().unwrap().push(Instant::now());
            ResponseTemplate::new(200)
                .set_body_json(chat_completion_json("queued completion"))
                .set_delay(self.delay)
        }
    }

    /// With one permit, the second `chat()` may only reach the daemon after
    /// the first response (including its 300 ms delay) completes.
    #[tokio::test]
    async fn ollama_concurrency_guard_serializes_requests() {
        let server = MockServer::start().await;
        let arrivals: Arc<Mutex<Vec<Instant>>> = Arc::default();
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(TrackingResponder {
                arrivals: arrivals.clone(),
                delay: Duration::from_millis(300),
            })
            .mount(&server)
            .await;

        let client = Arc::new(client_for(&server.uri()));
        let first = tokio::spawn({
            let c = client.clone();
            async move { c.chat(simple_request()).await }
        });
        let second = tokio::spawn({
            let c = client.clone();
            async move { c.chat(simple_request()).await }
        });
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();

        let arrivals = arrivals.lock().unwrap();
        assert_eq!(arrivals.len(), 2);
        let gap = arrivals[1].duration_since(arrivals[0]);
        assert!(
            gap >= Duration::from_millis(250),
            "second request arrived {gap:?} after the first; expected the semaphore to serialize them"
        );
    }

    // ── construction & configuration ────────────────────────────────────────

    fn account_cfg(model: Option<&str>) -> AccountAiConfig {
        AccountAiConfig {
            account_id: "acct-legal".into(),
            provider: AiProvider::Ollama,
            model: model.map(str::to_string),
            base_url: None,
            api_key_ref: None,
            daily_query_limit: 50,
            updated_at: 0,
        }
    }

    #[test]
    fn from_config_requires_model() {
        let err = OllamaClient::from_config(&account_cfg(None)).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
        let err = OllamaClient::from_config(&account_cfg(Some("   "))).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn from_config_builds_local_client_with_defaults() {
        let client = OllamaClient::from_config(&account_cfg(Some("llama3:8b"))).unwrap();
        assert_eq!(client.id(), AiProvider::Ollama);
        assert!(client.is_local());
        assert_eq!(client.base_url, DEFAULT_BASE_URL);
    }

    #[test]
    fn context_window_heuristics_and_fallback() {
        let w = |model: &str| {
            OllamaClient::new(model, None, None, 1)
                .unwrap()
                .context_window()
        };
        assert_eq!(w("llama3.1:8b"), 131_072);
        assert_eq!(w("llama3:8b"), 8_192);
        assert_eq!(w("qwen2.5:14b"), 32_768);
        assert_eq!(w("mistral:7b"), 32_768);
        assert_eq!(w("entirely-unknown-model"), 4_096);
        // Explicit override beats the heuristic.
        let client = OllamaClient::new("llama3:8b", None, Some(20_000), 1).unwrap();
        assert_eq!(client.context_window(), 20_000);
    }

    #[test]
    fn base_url_is_normalized() {
        let client =
            OllamaClient::new("llama3:8b", Some("http://localhost:11434/".into()), None, 1)
                .unwrap();
        assert_eq!(
            client.chat_url(),
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(client.tags_url(), "http://localhost:11434/api/tags");
    }

    /// dev/06 §6: local providers get the 120 s total budget (vs 60 s cloud);
    /// connect stays at 10 s, scanning at 2 s.
    #[test]
    fn local_timeouts_match_dev06() {
        assert_eq!(CONNECT_TIMEOUT, Duration::from_secs(10));
        assert_eq!(TOTAL_TIMEOUT, Duration::from_secs(120));
        assert_eq!(SCAN_CONNECT_TIMEOUT, Duration::from_secs(2));
    }

    #[test]
    fn model_matching_accepts_family_tags() {
        assert!(model_matches("llama3:8b", "llama3:8b"));
        assert!(model_matches("llama3:latest", "llama3"));
        assert!(!model_matches("llama3:8b", "llama3:70b"));
        assert!(!model_matches("qwen2.5:14b", "llama3"));
    }
}
