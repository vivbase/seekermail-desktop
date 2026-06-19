//! Anthropic Messages API adapter (T060, dev/06 §1–§2, §6).
//!
//! Maps the neutral `ChatRequest`/`ChatResponse` contract onto
//! `POST {base}/v1/messages`. The Anthropic wire shape differs from OpenAI in
//! four ways this file owns end-to-end: the system preamble is a **top-level**
//! `system` field (not a message), response `content` is an **array** of typed
//! blocks, auth uses the `x-api-key` header (not `Authorization: Bearer`), and
//! every call carries an `anthropic-version` header.
//!
//! Secret hygiene (09 §5): the API key is resolved from its [`KeySource`]
//! inside the call frame of `chat()`/`health()` only, lives in a [`Secret`]
//! (zeroized on drop, redacted in `Debug`), and never reaches a struct field,
//! a log line, or a `ProviderError` payload. Error payloads carry HTTP status
//! and short technical tags only — never response body text.
//!
//! ADR-0004 (no proxy): requests go directly to the endpoint the user
//! configured (default `https://api.anthropic.com`).
//!
//! Streaming (`chat_stream`, T061): with `"stream": true` the same endpoint
//! answers with typed SSE events. Only `content_block_delta` (whose
//! `delta.type == "text_delta"` text becomes the next delta) and
//! `message_stop` (which ends the stream) are acted on; `message_start`,
//! `content_block_start`, `content_block_stop`, `message_delta`, and `ping`
//! are skipped; an `error` event terminates the stream with a classified
//! [`ProviderError`] that never carries the event's message text (09 §5).
//! Cancellation is dropping the stream handle — the response body drops and
//! the HTTP connection closes (dev/06 §4).

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::ai::provider::{AiProviderClient, ChatDeltaStream, ProviderError};
use crate::ai::registry::AccountAiConfig;
use crate::ai::sse::{self, SseAction, SseEvent};
use crate::ai::types::{ChatRequest, ChatResponse, FinishReason, ProviderHealth, TokenUsage};
use crate::error::{AppError, AppResult};
use crate::keychain::{CredKind, Keychain, Secret};
use crate::types::AiProvider;

/// Default endpoint when the account has no `ai_base_url` override.
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// Pinned API revision sent as the `anthropic-version` header on every call.
/// Hardcoded as a single constant so an upgrade is a one-line change.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Connect timeout (dev/06 §6: connect 10 s).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Total request timeout (dev/06 §6: total 60 s for cloud providers).
const TOTAL_TIMEOUT: Duration = Duration::from_secs(60);

/// Model-prefix table for [`AiProviderClient::context_window`]. Claude 3+ /
/// Claude 4 generations all ship a 200k window (card §6).
const LARGE_CONTEXT_PREFIXES: &[&str] = &[
    "claude-3-5-sonnet",
    "claude-3-7-sonnet",
    "claude-sonnet-4",
    "claude-3-opus",
    "claude-opus-4",
    "claude-3-5-haiku",
    "claude-3-haiku",
    "claude-haiku",
];

/// Where the adapter obtains the API key at call time. Injectable so the
/// wiremock contract tests (and `probe`) never touch the OS Keychain.
enum KeySource {
    /// Production path: read `{account_id}:ai_api_key` from the OS Keychain
    /// inside the call frame; the plaintext drops when the call returns.
    Keychain {
        keychain: Keychain,
        account_id: Uuid,
    },
    /// One-shot key supplied by `verify_ai_provider` before it is persisted.
    Direct(Secret),
    /// No key at all (probe of a keyless gateway); the server decides.
    Anonymous,
}

/// `AiProviderClient` adapter for the Anthropic Messages API.
pub struct AnthropicClient {
    model: String,
    base_url: String,
    key_source: KeySource,
    /// Shared connection pool — reused across calls of this instance.
    http: reqwest::Client,
}

impl std::fmt::Debug for AnthropicClient {
    /// Identifier-only `Debug`: never prints endpoints-with-keys, key
    /// sources, or any request/response content (09 Â§5).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicClient")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl AnthropicClient {
    fn new(model: String, base_url: Option<String>, key_source: KeySource) -> Self {
        let base_url = base_url
            .filter(|u| !u.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string();
        // rustls-backed builder: construction only fails if the TLS backend
        // cannot initialize, which is unrecoverable for the whole app anyway.
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(TOTAL_TIMEOUT)
            .build()
            .expect("reqwest client construction must not fail with rustls");
        Self {
            model,
            base_url,
            key_source,
            http,
        }
    }

    /// Build the per-account adapter from its `account_ai_settings` row.
    /// Registered with the `AiRegistry` as the `anthropic` `ProviderFactory`.
    pub fn from_config(cfg: &AccountAiConfig, keychain: Keychain) -> AppResult<Arc<Self>> {
        let model = cfg
            .model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .ok_or_else(|| AppError::Validation("anthropic provider requires a model name".into()))?
            .to_string();
        let account_id = Uuid::parse_str(&cfg.account_id)
            .map_err(|_| AppError::Validation("account id is not a valid uuid".into()))?;
        Ok(Arc::new(Self::new(
            model,
            cfg.base_url.clone(),
            KeySource::Keychain {
                keychain,
                account_id,
            },
        )))
    }

    /// List the model ids the endpoint exposes via `GET {base}/v1/models`.
    /// Backs the `list_cloud_models` config command (the add-cloud-provider
    /// model picker, T068) and the connection-test probe below. The transient
    /// key is held in a [`Secret`] for this one request and dropped on return;
    /// error payloads carry only endpoint kind / HTTP status (09 §5).
    pub async fn list_models(
        api_key: Option<&str>,
        base_url: Option<&str>,
    ) -> Result<Vec<String>, ProviderError> {
        let base = base_url
            .filter(|u| !u.trim().is_empty())
            .map(|u| u.trim_end_matches('/').to_string())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let url = format!("{base}/v1/models");
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(TOTAL_TIMEOUT)
            .build()
            .map_err(|_| ProviderError::Unreachable("http client init failed".into()))?;

        let mut request = http
            .get(&url)
            .header("anthropic-version", ANTHROPIC_VERSION);
        if let Some(k) = api_key {
            // Wrapped in `Secret` so the plaintext zeroizes when this block ends.
            let secret = Secret::new(k);
            request = request.header("x-api-key", secret.expose());
        }
        let response = request.send().await.map_err(map_transport_error)?;

        let response = ensure_success(response).await?;
        let text = response
            .text()
            .await
            .map_err(|_| ProviderError::BadResponse("failed reading models list body".into()))?;
        let parsed: WireModelsList = serde_json::from_str(&text)
            .map_err(|_| ProviderError::BadResponse("models list did not parse as json".into()))?;
        let mut ids: Vec<String> = parsed
            .data
            .into_iter()
            .map(|m| m.id)
            .filter(|id| !id.is_empty())
            .collect();
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    /// One-shot reachability + auth probe for `verify_ai_provider` (02 §H),
    /// run against a key/endpoint the user has typed but not yet saved.
    /// Cross-card signature convention — `commands::ai` calls this directly.
    ///
    /// Verification reads the model catalog (`GET /v1/models`) rather than
    /// issuing a Messages call: it exercises DNS/TLS/auth end to end, spends no
    /// tokens, and confirms the model id by catalog membership. Endpoints with
    /// no usable catalog (404/parse) fall back to the one-shot Messages probe;
    /// `Auth`/`RateLimited`/`Unreachable` are conclusive and surface at once.
    pub async fn probe(
        model: &str,
        api_key: Option<&str>,
        base_url: Option<&str>,
    ) -> Result<ProviderHealth, ProviderError> {
        let started = Instant::now();
        match Self::list_models(api_key, base_url).await {
            Ok(models) => {
                let latency_ms = elapsed_ms(started);
                if models.iter().any(|m| m == model) {
                    Ok(ProviderHealth {
                        ok: true,
                        model_name: Some(model.to_string()),
                        latency_ms,
                    })
                } else {
                    // Catalog loaded but does not list this id (status only).
                    Err(ProviderError::BadResponse(
                        "http 404 (model not in catalog)".into(),
                    ))
                }
            }
            Err(
                err @ (ProviderError::Auth
                | ProviderError::RateLimited { .. }
                | ProviderError::Unreachable(_)),
            ) => Err(err),
            // Reachable but no usable `/v1/models`: fall back to the Messages probe.
            Err(_) => {
                let key_source = match api_key {
                    Some(key) => KeySource::Direct(Secret::new(key)),
                    None => KeySource::Anonymous,
                };
                let client = Self::new(model.to_string(), base_url.map(str::to_string), key_source);
                client.health().await
            }
        }
    }

    /// Resolve the API key for one call. The returned [`Secret`] stays in the
    /// caller's frame and zeroizes on drop. A keychain-backed source with no
    /// stored item is an auth failure — the settings must be re-entered.
    fn resolve_key(&self) -> Result<Option<Secret>, ProviderError> {
        match &self.key_source {
            KeySource::Direct(secret) => Ok(Some(secret.clone())),
            KeySource::Anonymous => Ok(None),
            KeySource::Keychain {
                keychain,
                account_id,
            } => match keychain.get(account_id, CredKind::AiApiKey) {
                Ok(Some(secret)) => Ok(Some(secret)),
                // Missing item or denied access: either way the configured
                // credential is unusable — same user remedy as a 401.
                Ok(None) | Err(_) => Err(ProviderError::Auth),
            },
        }
    }

    /// Build and POST one Messages API request and classify its status; shared
    /// by the non-streaming and streaming paths so an initial SSE response
    /// failure maps exactly like a `chat()` failure (card §3). The key lives
    /// only in this frame; dropped (and zeroized) on return.
    async fn post_messages(&self, body: &Value) -> Result<(reqwest::Response, u32), ProviderError> {
        let api_key = self.resolve_key()?;
        let url = format!("{}/v1/messages", self.base_url);

        let started = Instant::now();
        let mut request = self
            .http
            .post(&url)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(body);
        if let Some(key) = api_key.as_ref() {
            request = request.header("x-api-key", key.expose());
        }
        let response = request.send().await.map_err(map_transport_error)?;
        let latency_ms = elapsed_ms(started);

        // Status code and latency only — never headers, never the key (09 §5).
        tracing::debug!(
            event = "anthropic_messages_response",
            status = response.status().as_u16(),
            latency_ms = latency_ms,
            "anthropic messages call completed"
        );

        Ok((ensure_success(response).await?, latency_ms))
    }

    /// POST one Messages API body and return the parsed response + latency.
    /// All status/transport/parse classification lives here so `chat` and
    /// `health` share one error model.
    async fn send_messages(&self, body: Value) -> Result<(MessagesResponse, u32), ProviderError> {
        let (response, latency_ms) = self.post_messages(&body).await?;
        let text = response
            .text()
            .await
            .map_err(|_| ProviderError::BadResponse("failed reading response body".into()))?;
        let parsed: MessagesResponse = serde_json::from_str(&text)
            .map_err(|_| ProviderError::BadResponse("unparseable messages response".into()))?;
        Ok((parsed, latency_ms))
    }
}

/// Map a non-2xx Messages API status to the provider error model (dev/06 §6).
/// Error payloads carry the status and a fixed tag only; the response body is
/// inspected solely to classify the 400 context class and never leaves this
/// function (09 §5).
async fn ensure_success(response: reqwest::Response) -> Result<reqwest::Response, ProviderError> {
    let status = response.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(ProviderError::Auth);
    }
    if status.as_u16() == 429 {
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        return Err(ProviderError::RateLimited { retry_after });
    }
    if status.as_u16() == 400 {
        let text = response.text().await.unwrap_or_default();
        if is_context_too_long(&text) {
            return Err(ProviderError::ContextTooLong);
        }
        return Err(ProviderError::BadResponse(
            "http 400 from messages endpoint".into(),
        ));
    }
    if !status.is_success() {
        return Err(ProviderError::BadResponse(format!(
            "http {} from messages endpoint",
            status.as_u16()
        )));
    }
    Ok(response)
}

#[async_trait]
impl AiProviderClient for AnthropicClient {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let body = wire_body(&req);
        let (parsed, latency_ms) = self.send_messages(body).await?;

        let text = parsed
            .content
            .into_iter()
            .find(|block| block.kind == "text")
            .map(|block| block.text)
            .ok_or_else(|| ProviderError::BadResponse("empty content array".into()))?;

        let model_echo = if parsed.model.is_empty() {
            req.model.clone()
        } else {
            parsed.model
        };

        Ok(ChatResponse {
            text,
            finish: map_stop_reason(parsed.stop_reason.as_deref()),
            usage: TokenUsage {
                prompt_tokens: parsed.usage.input_tokens,
                completion_tokens: parsed.usage.output_tokens,
            },
            model_echo,
            latency_ms,
        })
    }

    async fn chat_stream(&self, req: ChatRequest) -> Result<ChatDeltaStream, ProviderError> {
        // Identifiers only — no key, no prompt, no completion (09 §5).
        tracing::debug!(
            event = "anthropic_chat_stream_request",
            model = %req.model,
            request_id = %req.request_id,
            purpose = req.purpose.as_str(),
            "opening streaming messages call"
        );

        let mut body = wire_body(&req);
        body["stream"] = json!(true);
        let (response, _latency_ms) = self.post_messages(&body).await?;

        // The response body moves into the stream state; dropping the returned
        // stream drops it and closes the HTTP connection (dev/06 §4).
        let byte_stream = response
            .bytes_stream()
            .map(|chunk| chunk.map(|b| b.to_vec()))
            .boxed();
        Ok(sse::delta_stream(
            byte_stream,
            map_transport_error,
            anthropic_stream_action,
        ))
    }

    async fn health(&self) -> Result<ProviderHealth, ProviderError> {
        // Minimal probe: one user turn, one output token (card §3).
        let body = json!({
            "model": self.model,
            "max_tokens": 1,
            "temperature": 0.0,
            "messages": [{
                "role": "user",
                "content": [{ "type": "text", "text": "hello" }],
            }],
            "stream": false,
        });
        let (parsed, latency_ms) = self.send_messages(body).await?;
        let model_name = if parsed.model.is_empty() {
            Some(self.model.clone())
        } else {
            Some(parsed.model)
        };
        Ok(ProviderHealth {
            ok: true,
            model_name,
            latency_ms,
        })
    }

    fn id(&self) -> AiProvider {
        AiProvider::Anthropic
    }

    fn context_window(&self) -> usize {
        context_window_for(&self.model)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Wire mapping helpers (neutral contract ⇄ Anthropic Messages shape)
// ─────────────────────────────────────────────────────────────────────────────

/// Neutral `ChatRequest` → Messages API request body. `system` is a top-level
/// field (never a message) and message content is an array of text blocks —
/// the two structural deltas vs. OpenAI. `purpose`/`request_id` are local
/// audit fields and are deliberately never serialized (dev/06 §2.1).
fn wire_body(req: &ChatRequest) -> Value {
    let messages: Vec<Value> = req
        .messages
        .iter()
        .map(|m| {
            json!({
                "role": m.role.as_str(),
                "content": [{ "type": "text", "text": m.content }],
            })
        })
        .collect();

    let mut body = json!({
        "model": req.model,
        "max_tokens": req.max_tokens,
        "temperature": req.temperature,
        "messages": messages,
        "stream": false,
    });
    if !req.system.is_empty() {
        body["system"] = json!(req.system);
    }
    if !req.stop.is_empty() {
        body["stop_sequences"] = json!(req.stop);
    }
    body
}

/// Classify one reassembled SSE event from the Messages stream (T061 card §3).
///
/// * `content_block_delta` with `delta.type == "text_delta"` → the delta text;
/// * `message_stop` → graceful end of stream;
/// * `error` → terminate with a [`ProviderError`] classified from the event's
///   `error.type` **only** — the event's message text never enters the error
///   value (09 §5);
/// * everything else (`message_start`, `content_block_start`,
///   `content_block_stop`, `message_delta`, `ping`, unknown types) → skipped.
fn anthropic_stream_action(event: &SseEvent) -> SseAction {
    match event.event.as_deref() {
        Some("content_block_delta") => {
            match serde_json::from_str::<WireStreamDeltaEvent>(&event.data) {
                Ok(parsed) => {
                    if parsed.delta.kind == "text_delta" && !parsed.delta.text.is_empty() {
                        SseAction::Delta(parsed.delta.text)
                    } else {
                        // Non-text deltas (e.g. input_json_delta) carry nothing
                        // for a draft body — skip rather than fail the stream.
                        SseAction::Skip
                    }
                }
                Err(_) => SseAction::Fail(ProviderError::BadResponse(
                    "unparseable content_block_delta event".into(),
                )),
            }
        }
        Some("message_stop") => SseAction::End,
        Some("error") => {
            let kind = serde_json::from_str::<StreamErrorEnvelope>(&event.data)
                .map(|envelope| envelope.error.kind)
                .unwrap_or_default();
            SseAction::Fail(match kind.as_str() {
                "rate_limit_error" => ProviderError::RateLimited { retry_after: None },
                "authentication_error" | "permission_error" => ProviderError::Auth,
                "overloaded_error" => {
                    ProviderError::Unreachable("provider overloaded (stream error event)".into())
                }
                _ => ProviderError::BadResponse("error event on messages stream".into()),
            })
        }
        _ => SseAction::Skip,
    }
}

/// Anthropic `stop_reason` → neutral [`FinishReason`]. Unknown values degrade
/// to `Stop` so a new vendor enum value never breaks draft delivery.
fn map_stop_reason(reason: Option<&str>) -> FinishReason {
    match reason {
        Some("end_turn") | Some("stop_sequence") => FinishReason::Stop,
        Some("max_tokens") => FinishReason::Length,
        Some("refusal") | Some("content_filter") | Some("content_filtered") => {
            FinishReason::ContentFilter
        }
        _ => FinishReason::Stop,
    }
}

/// Model token budget by slug prefix; conservative 100k fallback for unknown
/// Claude models (card §6 — far more realistic than a small-model default).
fn context_window_for(model: &str) -> usize {
    if LARGE_CONTEXT_PREFIXES.iter().any(|p| model.starts_with(p)) {
        200_000
    } else {
        100_000
    }
}

/// Classify an HTTP 400 error body as a context-window overflow. Matches the
/// documented Anthropic phrasing ("prompt is too long") plus the max_tokens ×
/// context-limit variant; the body is read for this check only and never
/// stored or logged.
fn is_context_too_long(body_text: &str) -> bool {
    let envelope: ErrorEnvelope = match serde_json::from_str(body_text) {
        Ok(e) => e,
        Err(_) => return false,
    };
    let message = envelope.error.message.to_ascii_lowercase();
    message.contains("prompt is too long")
        || (message.contains("max_tokens") && message.contains("context"))
}

/// reqwest transport failure → [`ProviderError`] with a short technical tag.
/// The reqwest error string can embed the full request URL, so it is reduced
/// to a fixed-vocabulary tag here.
fn map_transport_error(err: reqwest::Error) -> ProviderError {
    let tag = if err.is_timeout() {
        "request timed out"
    } else if err.is_connect() {
        "connection failed"
    } else if err.is_builder() {
        "invalid endpoint url"
    } else {
        "network error"
    };
    ProviderError::Unreachable(tag.into())
}

/// Elapsed wall time as `u32` milliseconds, saturating.
fn elapsed_ms(started: Instant) -> u32 {
    u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX)
}

// ─────────────────────────────────────────────────────────────────────────────
// Anthropic response shapes (deserialization only; lenient by default so a
// missing optional field degrades instead of erroring)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct MessagesResponse {
    #[serde(default)]
    content: Vec<ContentBlock>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: WireUsage,
    #[serde(default)]
    model: String,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    text: String,
}

#[derive(Debug, Default, Deserialize)]
struct WireUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

/// `GET /v1/models` catalog: `{"data":[{"type":"model","id":…}],"has_more":…}`.
#[derive(Debug, Default, Deserialize)]
struct WireModelsList {
    #[serde(default)]
    data: Vec<WireModelEntry>,
}

#[derive(Debug, Default, Deserialize)]
struct WireModelEntry {
    #[serde(default)]
    id: String,
}

/// Anthropic error envelope: `{"type":"error","error":{"type":…,"message":…}}`.
#[derive(Debug, Default, Deserialize)]
struct ErrorEnvelope {
    #[serde(default)]
    error: ErrorBody,
}

#[derive(Debug, Default, Deserialize)]
struct ErrorBody {
    #[serde(default)]
    message: String,
}

// ── Anthropic SSE event shapes (T061) ────────────────────────────────────────

/// `event: content_block_delta` payload — only the delta itself matters.
#[derive(Debug, Default, Deserialize)]
struct WireStreamDeltaEvent {
    #[serde(default)]
    delta: WireStreamDelta,
}

#[derive(Debug, Default, Deserialize)]
struct WireStreamDelta {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    text: String,
}

/// `event: error` payload. Only `error.type` is read — the message text is
/// deliberately not deserialized so it can never reach an error value (09 §5).
#[derive(Debug, Default, Deserialize)]
struct StreamErrorEnvelope {
    #[serde(default)]
    error: StreamErrorBody,
}

#[derive(Debug, Default, Deserialize)]
struct StreamErrorBody {
    #[serde(rename = "type", default)]
    kind: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests — wiremock contract tests, zero network beyond localhost, zero spend.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::types::Capability;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TEST_KEY: &str = "sk-ant-contract-test-key";
    const TEST_MODEL: &str = "claude-3-5-sonnet-latest";

    fn client_for(server: &MockServer) -> AnthropicClient {
        AnthropicClient::new(
            TEST_MODEL.to_string(),
            Some(server.uri()),
            KeySource::Direct(Secret::new(TEST_KEY)),
        )
    }

    fn sample_request() -> ChatRequest {
        let mut req =
            ChatRequest::simple(TEST_MODEL, "Summarize this thread.", Capability::Summarize);
        req.system = "You are a terse summarizer.".to_string();
        req
    }

    #[tokio::test]
    async fn anthropic_chat_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", TEST_KEY))
            .and(header("anthropic-version", ANTHROPIC_VERSION))
            // `system` must be a top-level field and content an array of
            // typed text blocks — the two Anthropic-specific wire deltas.
            .and(body_partial_json(serde_json::json!({
                "model": TEST_MODEL,
                "system": "You are a terse summarizer.",
                "stream": false,
                "messages": [{
                    "role": "user",
                    "content": [{ "type": "text", "text": "Summarize this thread." }],
                }],
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "msg_01",
                "type": "message",
                "role": "assistant",
                "model": "claude-3-5-sonnet-20241022",
                "content": [{ "type": "text", "text": "hello" }],
                "stop_reason": "end_turn",
                "usage": { "input_tokens": 5, "output_tokens": 3 },
            })))
            .expect(1)
            .mount(&server)
            .await;

        let resp = client_for(&server).chat(sample_request()).await.unwrap();
        assert_eq!(resp.text, "hello");
        assert_eq!(resp.finish, FinishReason::Stop);
        assert_eq!(resp.usage.prompt_tokens, 5);
        assert_eq!(resp.usage.completion_tokens, 3);
        assert_eq!(resp.model_echo, "claude-3-5-sonnet-20241022");
    }

    #[tokio::test]
    async fn anthropic_chat_auth_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "type": "error",
                "error": { "type": "authentication_error", "message": "invalid x-api-key" },
            })))
            .mount(&server)
            .await;

        let err = client_for(&server)
            .chat(sample_request())
            .await
            .unwrap_err();
        assert_eq!(err, ProviderError::Auth);
    }

    #[tokio::test]
    async fn anthropic_chat_rate_limited_with_retry_after() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "30")
                    .set_body_json(serde_json::json!({
                        "type": "error",
                        "error": { "type": "rate_limit_error", "message": "rate limited" },
                    })),
            )
            .mount(&server)
            .await;

        let err = client_for(&server)
            .chat(sample_request())
            .await
            .unwrap_err();
        assert_eq!(
            err,
            ProviderError::RateLimited {
                retry_after: Some(Duration::from_secs(30)),
            }
        );
    }

    #[tokio::test]
    async fn anthropic_chat_context_too_long() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "type": "error",
                "error": {
                    "type": "invalid_request_error",
                    "message": "prompt is too long: 210042 tokens > 200000 maximum",
                },
            })))
            .mount(&server)
            .await;

        let err = client_for(&server)
            .chat(sample_request())
            .await
            .unwrap_err();
        assert_eq!(err, ProviderError::ContextTooLong);
    }

    #[tokio::test]
    async fn anthropic_health_ok() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(body_partial_json(serde_json::json!({
                "model": TEST_MODEL,
                "max_tokens": 1,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "msg_02",
                "type": "message",
                "role": "assistant",
                "model": "claude-3-5-sonnet-20241022",
                "content": [{ "type": "text", "text": "H" }],
                "stop_reason": "max_tokens",
                "usage": { "input_tokens": 8, "output_tokens": 1 },
            })))
            .expect(1)
            .mount(&server)
            .await;

        let health = client_for(&server).health().await.unwrap();
        assert!(health.ok);
        assert_eq!(
            health.model_name.as_deref(),
            Some("claude-3-5-sonnet-20241022")
        );
    }

    #[tokio::test]
    async fn anthropic_probe_uses_direct_key_and_base_url() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", TEST_KEY))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "model": "claude-3-5-sonnet-20241022",
                "content": [{ "type": "text", "text": "H" }],
                "stop_reason": "end_turn",
                "usage": { "input_tokens": 8, "output_tokens": 1 },
            })))
            .expect(1)
            .mount(&server)
            .await;

        let health = AnthropicClient::probe(TEST_MODEL, Some(TEST_KEY), Some(&server.uri()))
            .await
            .unwrap();
        assert!(health.ok);
    }

    // ── model catalog + catalog-based probe (T068) ──────────────────────────

    #[tokio::test]
    async fn anthropic_list_models_parses_catalog() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("x-api-key", TEST_KEY))
            .and(header("anthropic-version", ANTHROPIC_VERSION))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    { "type": "model", "id": "claude-sonnet-4-6" },
                    { "type": "model", "id": "claude-opus-4-8" },
                ],
                "has_more": false,
            })))
            .mount(&server)
            .await;

        let models = AnthropicClient::list_models(Some(TEST_KEY), Some(&server.uri()))
            .await
            .unwrap();
        // Sorted + deduped.
        assert_eq!(models, vec!["claude-opus-4-8", "claude-sonnet-4-6"]);
    }

    #[tokio::test]
    async fn anthropic_probe_ok_via_catalog_membership() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [ { "type": "model", "id": TEST_MODEL } ],
            })))
            .mount(&server)
            .await;

        let health = AnthropicClient::probe(TEST_MODEL, Some(TEST_KEY), Some(&server.uri()))
            .await
            .unwrap();
        assert!(health.ok);
        assert_eq!(health.model_name.as_deref(), Some(TEST_MODEL));
    }

    #[tokio::test]
    async fn anthropic_connection_refused_maps_to_unreachable() {
        // Port 1 on loopback is essentially guaranteed closed; the connect is
        // refused before any HTTP exchange.
        let client = AnthropicClient::new(
            TEST_MODEL.to_string(),
            Some("http://127.0.0.1:1".to_string()),
            KeySource::Direct(Secret::new(TEST_KEY)),
        );
        let err = client.chat(sample_request()).await.unwrap_err();
        assert!(matches!(err, ProviderError::Unreachable(_)));
    }

    #[tokio::test]
    async fn anthropic_errors_never_leak_key_or_body_text() {
        const BODY_MARKER: &str = "EXTREMELY-SENSITIVE-SERVER-BODY-TEXT";
        let server = MockServer::start().await;
        // A 400 that is NOT a context error, with a marker string in the body:
        // the classifier reads it, but the error payload must not carry it.
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "type": "error",
                "error": { "type": "invalid_request_error", "message": BODY_MARKER },
            })))
            .mount(&server)
            .await;

        let err = client_for(&server)
            .chat(sample_request())
            .await
            .unwrap_err();
        for rendered in [format!("{err}"), format!("{err:?}")] {
            assert!(!rendered.contains(TEST_KEY), "api key leaked: {rendered}");
            assert!(
                !rendered.contains(BODY_MARKER),
                "response body text leaked: {rendered}"
            );
        }
        // The status itself is the only payload detail.
        assert!(format!("{err}").contains("400"));
    }

    #[tokio::test]
    async fn anthropic_empty_content_is_bad_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "model": "claude-3-5-sonnet-20241022",
                "content": [],
                "stop_reason": "end_turn",
                "usage": { "input_tokens": 5, "output_tokens": 0 },
            })))
            .mount(&server)
            .await;

        let err = client_for(&server)
            .chat(sample_request())
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::BadResponse(_)));
    }

    // ── chat_stream (T061) ──────────────────────────────────────────────────

    fn sse_template(body: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_raw(body.as_bytes().to_vec(), "text/event-stream")
    }

    #[tokio::test]
    async fn anthropic_stream_delta_and_stop() {
        let sse_body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_03\",\"role\":\"assistant\"}}\n",
            "\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n",
            "\n",
            "event: ping\n",
            "data: {\"type\":\"ping\"}\n",
            "\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n",
            "\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"lo \"}}\n",
            "\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"there\"}}\n",
            "\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n",
            "\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":3}}\n",
            "\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n",
            "\n",
        );
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", TEST_KEY))
            .and(header("anthropic-version", ANTHROPIC_VERSION))
            .and(body_partial_json(serde_json::json!({ "stream": true })))
            .respond_with(sse_template(sse_body))
            .expect(1)
            .mount(&server)
            .await;

        let mut stream = client_for(&server)
            .chat_stream(sample_request())
            .await
            .unwrap();
        let mut deltas = Vec::new();
        while let Some(item) = stream.next().await {
            deltas.push(item.unwrap());
        }

        // Only the three text deltas surface; bookkeeping events are skipped
        // and `message_stop` closes the stream.
        assert_eq!(deltas.len(), 3);
        let text: String = deltas.iter().map(|d| d.text.as_str()).collect();
        assert_eq!(text, "Hello there");
        for (i, delta) in deltas.iter().enumerate() {
            assert_eq!(delta.index, i, "delta ordinals must be 0-based");
        }
    }

    /// An `error` event terminates the stream with a classified error whose
    /// payload never contains the event's message text (09 §5; deviation from
    /// the card's literal `BadResponse(event.error.message)` — content-free by
    /// design).
    #[tokio::test]
    async fn anthropic_stream_error_event_classified_without_leak() {
        const EVENT_MARKER: &str = "OVERLOADED-EVENT-MESSAGE-3318";
        let sse_body = format!(
            "event: content_block_delta\n\
             data: {{\"type\":\"content_block_delta\",\"delta\":{{\"type\":\"text_delta\",\"text\":\"ok\"}}}}\n\
             \n\
             event: error\n\
             data: {{\"type\":\"error\",\"error\":{{\"type\":\"overloaded_error\",\"message\":\"{EVENT_MARKER}\"}}}}\n\
             \n"
        );
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(sse_template(&sse_body))
            .mount(&server)
            .await;

        let mut stream = client_for(&server)
            .chat_stream(sample_request())
            .await
            .unwrap();
        assert_eq!(stream.next().await.unwrap().unwrap().text, "ok");
        let err = stream.next().await.unwrap().unwrap_err();
        assert!(matches!(err, ProviderError::Unreachable(_)));
        let rendered = format!("{err} {err:?}");
        assert!(
            !rendered.contains(EVENT_MARKER),
            "leaked event text: {rendered}"
        );
        assert!(
            stream.next().await.is_none(),
            "stream must terminate after the error"
        );
    }

    /// Mid-stream malformed delta JSON → one `Err(BadResponse)` with no
    /// payload leak, then termination.
    #[tokio::test]
    async fn anthropic_stream_malformed_delta_fails_without_leak() {
        const PAYLOAD_MARKER: &str = "PRIVATE-DELTA-FRAGMENT-6604";
        let sse_body = format!(
            "event: content_block_delta\n\
             data: {{broken json {PAYLOAD_MARKER}\n\
             \n"
        );
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(sse_template(&sse_body))
            .mount(&server)
            .await;

        let mut stream = client_for(&server)
            .chat_stream(sample_request())
            .await
            .unwrap();
        let err = stream.next().await.unwrap().unwrap_err();
        assert!(matches!(err, ProviderError::BadResponse(_)));
        let rendered = format!("{err} {err:?}");
        assert!(
            !rendered.contains(PAYLOAD_MARKER),
            "leaked payload: {rendered}"
        );
        assert!(stream.next().await.is_none());
    }

    /// An initial-response failure on the streaming path classifies exactly
    /// like a `chat()` failure (card §3).
    #[tokio::test]
    async fn anthropic_stream_initial_rate_limit_maps_like_chat() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "15")
                    .set_body_json(serde_json::json!({
                        "type": "error",
                        "error": { "type": "rate_limit_error", "message": "rate limited" },
                    })),
            )
            .mount(&server)
            .await;

        let err = client_for(&server)
            .chat_stream(sample_request())
            .await
            .err()
            .unwrap();
        assert_eq!(
            err,
            ProviderError::RateLimited {
                retry_after: Some(Duration::from_secs(15)),
            }
        );
    }

    #[test]
    fn anthropic_identity_and_context_window() {
        let client = AnthropicClient::new(
            "claude-3-5-sonnet-latest".to_string(),
            None,
            KeySource::Anonymous,
        );
        assert_eq!(client.id(), AiProvider::Anthropic);
        assert_eq!(client.context_window(), 200_000);

        assert_eq!(context_window_for("claude-3-opus-20240229"), 200_000);
        assert_eq!(context_window_for("claude-opus-4-1"), 200_000);
        assert_eq!(context_window_for("claude-sonnet-4-5"), 200_000);
        assert_eq!(context_window_for("claude-3-haiku-20240307"), 200_000);
        assert_eq!(context_window_for("claude-2.1"), 100_000);
        assert_eq!(context_window_for("entirely-unknown-model"), 100_000);
    }

    #[test]
    fn anthropic_stop_reason_mapping() {
        assert_eq!(map_stop_reason(Some("end_turn")), FinishReason::Stop);
        assert_eq!(map_stop_reason(Some("stop_sequence")), FinishReason::Stop);
        assert_eq!(map_stop_reason(Some("max_tokens")), FinishReason::Length);
        assert_eq!(
            map_stop_reason(Some("refusal")),
            FinishReason::ContentFilter
        );
        assert_eq!(map_stop_reason(Some("something_new")), FinishReason::Stop);
        assert_eq!(map_stop_reason(None), FinishReason::Stop);
    }

    #[test]
    fn anthropic_from_config_requires_model() {
        let cfg = AccountAiConfig {
            account_id: Uuid::new_v4().to_string(),
            provider: AiProvider::Anthropic,
            model: None,
            base_url: None,
            api_key_ref: None,
            daily_query_limit: 50,
            updated_at: 0,
        };
        let err = AnthropicClient::from_config(&cfg, Keychain::new()).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn anthropic_from_config_builds_client() {
        let cfg = AccountAiConfig {
            account_id: Uuid::new_v4().to_string(),
            provider: AiProvider::Anthropic,
            model: Some(TEST_MODEL.to_string()),
            base_url: Some("https://gateway.example.com/".to_string()),
            api_key_ref: Some("ai_api_key".to_string()),
            daily_query_limit: 50,
            updated_at: 0,
        };
        let client = AnthropicClient::from_config(&cfg, Keychain::new()).unwrap();
        assert_eq!(client.id(), AiProvider::Anthropic);
        // Trailing slash on the override is normalized away.
        assert_eq!(client.base_url, "https://gateway.example.com");
        assert_eq!(client.context_window(), 200_000);
    }
}
