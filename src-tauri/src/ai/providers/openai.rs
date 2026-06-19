//! `OpenAiClient` — OpenAI Chat Completions adapter (T059, dev/06 §1, §2, §6).
//!
//! Maps the neutral [`ChatRequest`]/[`ChatResponse`] contract onto
//! `POST {base}/v1/chat/completions`. A custom `base_url` reaches
//! OpenAI-compatible gateways (Gemini-compatible paths, Azure-style fronts,
//! self-hosted shims) without a dedicated adapter (dev/06 §1).
//!
//! Key discipline (ADR-0004, F_F1 §4.2): the API key is read from the OS
//! Keychain inside the request call frame, injected into the `Authorization`
//! header, and dropped (zeroized via [`Secret`]) when the frame ends. It is
//! never persisted in a long-lived field, never logged, and never copied into
//! an error payload. The one sanctioned exception is [`KeySource::Direct`],
//! which exists for the one-shot `verify_ai_provider` probe and for tests —
//! the holding client is built, used once, and dropped within the same call.
//!
//! Streaming (`chat_stream`, T061): `"stream": true` turns the same endpoint
//! into `text/event-stream`; the shared [`crate::ai::sse`] parser reassembles
//! `data:` chunks, `choices[0].delta.content` becomes the next
//! [`crate::ai::types::ChatDelta`], and
//! `data: [DONE]` ends the stream. Cancellation is dropping the stream handle —
//! that drops the response body and closes the HTTP connection (dev/06 §4).

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{AUTHORIZATION, RETRY_AFTER};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ai::provider::{AiProviderClient, ChatDeltaStream, ProviderError};
use crate::ai::registry::AccountAiConfig;
use crate::ai::sse::{self, SseAction, SseEvent};
use crate::ai::types::{
    Capability, ChatRequest, ChatResponse, FinishReason, ProviderHealth, TokenUsage,
};
use crate::error::{AppError, AppResult};
use crate::keychain::{CredKind, Keychain, Secret};
use crate::types::AiProvider;

/// Default endpoint when the account has no custom `ai_base_url` (dev/06 §1).
const DEFAULT_BASE_URL: &str = "https://api.openai.com";

/// Connect timeout (F_F1 §4.5, dev/06 §6).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Total request timeout for cloud providers (F_F1 §4.5, dev/06 §6).
const TOTAL_TIMEOUT: Duration = Duration::from_secs(60);

/// Where this client obtains its API key at call time.
///
/// Production clients built by [`OpenAiClient::from_config`] always use
/// [`KeySource::Keychain`] so the key only ever lives in the OS Keychain.
/// [`KeySource::Direct`] backs the transient `verify_ai_provider` probe (the
/// user is testing a key *before* saving it) and the wiremock contract tests;
/// [`KeySource::NoAuth`] supports keyless OpenAI-compatible gateways.
enum KeySource {
    /// Read `{account_id}:ai_api_key` from the OS Keychain per request.
    Keychain(Uuid),
    /// A transient key held only for the lifetime of a one-shot client.
    Direct(Secret),
    /// No `Authorization` header (keyless local/compatible gateways).
    NoAuth,
}

/// OpenAI Chat Completions client. One instance per account configuration;
/// the registry caches it until the settings row changes (T058).
pub struct OpenAiClient {
    model: String,
    base_url: String,
    org_id: Option<String>,
    key_source: KeySource,
    keychain: Keychain,
    /// Shared connection pool; connect 10 s / total 60 s (dev/06 §6).
    http: reqwest::Client,
}

impl std::fmt::Debug for OpenAiClient {
    /// Identifier-only `Debug`: never prints endpoints-with-keys, key
    /// sources, or any request/response content (09 Â§5).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiClient")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl OpenAiClient {
    /// Build a Keychain-backed client. `api_key_ref` is the value of
    /// `account_ai_settings.ai_api_key_ref` — the account UUID string under
    /// which the key was stored (`Keychain::set(&uuid, CredKind::AiApiKey, …)`).
    pub fn new(
        model: String,
        api_key_ref: &str,
        base_url: Option<String>,
        org_id: Option<String>,
        keychain: Keychain,
    ) -> AppResult<Self> {
        let account_id = crate::util::parse_uuid(api_key_ref)?;
        Self::build(
            model,
            KeySource::Keychain(account_id),
            base_url,
            org_id,
            keychain,
        )
    }

    /// Factory entry point for [`crate::ai::registry::AiRegistry::register_factory`].
    /// Requires a configured model; the key reference is optional so keyless
    /// OpenAI-compatible gateways remain reachable.
    pub fn from_config(cfg: &AccountAiConfig, keychain: Keychain) -> AppResult<Arc<Self>> {
        let model = cfg.model.clone().ok_or_else(|| {
            AppError::Validation("an OpenAI model must be configured for this account".into())
        })?;
        let key_source = match cfg.api_key_ref.as_deref() {
            Some(reference) => KeySource::Keychain(crate::util::parse_uuid(reference)?),
            None => KeySource::NoAuth,
        };
        Ok(Arc::new(Self::build(
            model,
            key_source,
            cfg.base_url.clone(),
            None,
            keychain,
        )?))
    }

    fn build(
        model: String,
        key_source: KeySource,
        base_url: Option<String>,
        org_id: Option<String>,
        keychain: Keychain,
    ) -> AppResult<Self> {
        let http = http_client()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("http client init failed: {e}")))?;
        Ok(Self {
            model,
            base_url: base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            org_id,
            key_source,
            keychain,
            http,
        })
    }

    /// Resolve the key for one request. Keychain reads happen here, inside the
    /// caller's frame, so the plaintext never outlives the request (F_F1 §4.2).
    /// A missing or unreadable key is an auth-class failure: the user must
    /// re-enter provider settings (dev/06 §6).
    fn resolve_key(&self) -> Result<Option<Secret>, ProviderError> {
        match &self.key_source {
            KeySource::Keychain(account_id) => {
                match self.keychain.get(account_id, CredKind::AiApiKey) {
                    Ok(Some(secret)) => Ok(Some(secret)),
                    Ok(None) => Err(ProviderError::Auth),
                    Err(_) => Err(ProviderError::Auth),
                }
            }
            KeySource::Direct(secret) => Ok(Some(secret.clone())),
            KeySource::NoAuth => Ok(None),
        }
    }

    fn completions_url(&self) -> String {
        openai_endpoint(&self.base_url, "chat/completions")
    }

    /// Build and POST one Chat Completions request (`stream` toggles SSE),
    /// adapting the request shape to the endpoint until it is accepted, and
    /// returning an already-2xx response.
    ///
    /// No single parameter shape works everywhere (dev/06 §1): current OpenAI
    /// reasoning models (`gpt-5.x`, the o-series) require `max_completion_tokens`
    /// and reject a non-default `temperature`, while many OpenAI-compatible
    /// gateways only understand the legacy `max_tokens`. The first attempt is
    /// chosen by a model heuristic; on a 400 that names an incompatible
    /// parameter, the shape is adjusted and the request retried (bounded by
    /// [`MAX_PARAM_RETRIES`]). The 400 body is read only to decide the
    /// adjustment — its text never enters an error payload or a log line (09 §5).
    ///
    /// The key lives only inside this frame (F_F1 §4.2): it is resolved once,
    /// injected into the `Authorization` header, and dropped (zeroized) before
    /// the response is returned.
    async fn send_completions(
        &self,
        req: &ChatRequest,
        stream: bool,
    ) -> Result<reqwest::Response, ProviderError> {
        let key = self.resolve_key()?;
        let url = self.completions_url();
        let mut profile = WireProfile::for_model(&req.model);

        for _ in 0..MAX_PARAM_RETRIES {
            let body = build_completions_body(req, stream, profile);
            let mut http_req = self.http.post(&url).json(&body);
            if let Some(secret) = key.as_ref() {
                http_req = http_req.header(AUTHORIZATION, format!("Bearer {}", secret.expose()));
            }
            if let Some(org) = &self.org_id {
                http_req = http_req.header("OpenAI-Organization", org);
            }
            let resp = http_req.send().await.map_err(map_transport_err)?;
            match classify_completions_response(resp, profile).await {
                CompletionsOutcome::Success(resp) => {
                    drop(key);
                    return Ok(resp);
                }
                CompletionsOutcome::Error(err) => {
                    drop(key);
                    return Err(err);
                }
                // The endpoint rejected a parameter we can rename or drop — adjust
                // the shape and try again. No body text is retained.
                CompletionsOutcome::AdaptAndRetry(next) => profile = next,
            }
        }

        drop(key);
        // Bounded retries exhausted without an accepted parameter shape — status
        // only, never the body text (09 §5).
        Err(ProviderError::BadResponse(
            "http 400 (incompatible request parameters)".into(),
        ))
    }
}

/// Bounded number of parameter-shape adaptations per request (model heuristic +
/// at most a token-name flip and a temperature drop).
const MAX_PARAM_RETRIES: usize = 3;

/// Which Chat Completions parameter shape to send (dev/06 §1). The two axes that
/// vary across OpenAI and OpenAI-compatible endpoints: reasoning models
/// (`gpt-5.x`, the o-series) take `max_completion_tokens` and reject a custom
/// `temperature`; legacy models and many compatible gateways take `max_tokens`
/// and accept `temperature`.
#[derive(Clone, Copy)]
struct WireProfile {
    use_max_completion_tokens: bool,
    include_temperature: bool,
}

impl WireProfile {
    /// First-attempt shape chosen from the model id. A wrong guess is corrected
    /// by the 400-driven adaptation in `send_completions`, so it only needs to
    /// be right often enough to avoid an extra round-trip in the common case.
    fn for_model(model: &str) -> Self {
        let m = model.to_ascii_lowercase();
        let reasoning = m.starts_with("gpt-5")
            || m.starts_with("gpt-6")
            || m.starts_with("o1")
            || m.starts_with("o3")
            || m.starts_with("o4")
            || m.starts_with("o5");
        WireProfile {
            use_max_completion_tokens: reasoning,
            include_temperature: !reasoning,
        }
    }
}

/// Build the Chat Completions body for one attempt. `purpose`/`request_id` are
/// local audit fields and are deliberately never serialized (dev/06 §2.1).
fn build_completions_body<'a>(
    req: &'a ChatRequest,
    stream: bool,
    profile: WireProfile,
) -> WireChatBody<'a> {
    // System preamble travels as the leading "system" message (dev/06 §2.1).
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
    let (max_tokens, max_completion_tokens) = if profile.use_max_completion_tokens {
        (None, Some(req.max_tokens))
    } else {
        (Some(req.max_tokens), None)
    };
    WireChatBody {
        model: &req.model,
        messages,
        max_tokens,
        max_completion_tokens,
        temperature: profile.include_temperature.then_some(req.temperature),
        stop: &req.stop,
        stream,
    }
}

/// Outcome of inspecting the initial Chat Completions response.
enum CompletionsOutcome {
    /// 2xx — hand the response back to the caller.
    Success(reqwest::Response),
    /// A 400 named a parameter we can rename or drop — retry with this shape.
    AdaptAndRetry(WireProfile),
    /// A conclusive failure (status only; never body text — 09 §5).
    Error(ProviderError),
}

/// Classify the initial Chat Completions response, shared by the non-streaming
/// and streaming paths so an initial SSE failure classifies exactly like a
/// `chat()` failure (card §3). The 400 body is read only to (a) detect the
/// context-overflow class and (b) decide a parameter adaptation; its text never
/// leaves this function (09 §5).
async fn classify_completions_response(
    resp: reqwest::Response,
    profile: WireProfile,
) -> CompletionsOutcome {
    let status = resp.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return CompletionsOutcome::Error(ProviderError::Auth);
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        let retry_after = resp
            .headers()
            .get(RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        return CompletionsOutcome::Error(ProviderError::RateLimited { retry_after });
    }
    if status == StatusCode::BAD_REQUEST {
        let body = resp.text().await.unwrap_or_default().to_ascii_lowercase();
        if body.contains("context_length_exceeded") || body.contains("maximum context length") {
            return CompletionsOutcome::Error(ProviderError::ContextTooLong);
        }
        if let Some(next) = adapt_profile(&body, profile) {
            return CompletionsOutcome::AdaptAndRetry(next);
        }
        return CompletionsOutcome::Error(ProviderError::BadResponse("http 400".into()));
    }
    if !status.is_success() {
        return CompletionsOutcome::Error(ProviderError::BadResponse(format!(
            "http {}",
            status.as_u16()
        )));
    }
    CompletionsOutcome::Success(resp)
}

/// Decide how to adjust the request shape from a 400 body that names an
/// unsupported parameter. Returns the adjusted profile when a rename/drop is
/// possible, or `None` when the 400 is not a parameter-compatibility error.
/// Only the lowercased body is inspected here; nothing is retained (09 §5).
fn adapt_profile(body_lower: &str, profile: WireProfile) -> Option<WireProfile> {
    let mut next = profile;
    let mut changed = false;

    if body_lower.contains("max_completion_tokens") {
        if !profile.use_max_completion_tokens {
            // "use 'max_completion_tokens' instead" — adopt the modern name.
            next.use_max_completion_tokens = true;
            changed = true;
        } else if body_lower.contains("unrecognized")
            || body_lower.contains("unknown")
            || body_lower.contains("unexpected")
            || body_lower.contains("unsupported")
            || body_lower.contains("invalid")
            || body_lower.contains("not permitted")
        {
            // The gateway does not accept the modern name — fall back to max_tokens.
            next.use_max_completion_tokens = false;
            changed = true;
        }
    }

    if profile.include_temperature
        && body_lower.contains("temperature")
        && (body_lower.contains("unsupported")
            || body_lower.contains("does not support")
            || body_lower.contains("only the default")
            || body_lower.contains("not supported")
            || body_lower.contains("must be"))
    {
        // Reasoning models accept only the default temperature — drop the field.
        next.include_temperature = false;
        changed = true;
    }

    changed.then_some(next)
}

/// Map a non-2xx status to the provider error model (dev/06 §6) — used by the
/// model-catalog read (`list_models`). Error payloads carry the status only;
/// response bodies never leave this function (09 §5).
async fn ensure_success(resp: reqwest::Response) -> Result<reqwest::Response, ProviderError> {
    let status = resp.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(ProviderError::Auth);
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        let retry_after = resp
            .headers()
            .get(RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        return Err(ProviderError::RateLimited { retry_after });
    }
    if status == StatusCode::BAD_REQUEST {
        // The body is inspected only to classify the 400; its text never
        // leaves this frame and never enters an error payload (09 §5).
        let body_text = resp.text().await.unwrap_or_default();
        if body_text.contains("context_length_exceeded") {
            return Err(ProviderError::ContextTooLong);
        }
        return Err(ProviderError::BadResponse("http 400".into()));
    }
    if !status.is_success() {
        // Status-only detail — response bodies stay out of errors (09 §5).
        return Err(ProviderError::BadResponse(format!(
            "http {}",
            status.as_u16()
        )));
    }
    Ok(resp)
}

/// Classify one reassembled SSE event from the Chat Completions stream
/// (card §3): `[DONE]` terminates, `choices[0].delta.content` is the delta
/// text, role-only/empty chunks are skipped, and a malformed payload fails
/// with a fixed tag — the payload text never enters the error (09 §5).
fn openai_stream_action(event: &SseEvent) -> SseAction {
    let data = event.data.trim();
    if data.is_empty() {
        return SseAction::Skip;
    }
    if data == "[DONE]" {
        return SseAction::End;
    }
    match serde_json::from_str::<WireStreamChunk>(data) {
        Ok(chunk) => match chunk
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.delta.content)
            .filter(|text| !text.is_empty())
        {
            Some(text) => SseAction::Delta(text),
            None => SseAction::Skip,
        },
        Err(_) => SseAction::Fail(ProviderError::BadResponse(
            "unparseable sse delta chunk".into(),
        )),
    }
}

/// List the model ids the endpoint exposes via `GET {base}/v1/models` (the
/// OpenAI-compatible model catalog). Backs both the `list_cloud_models` config
/// command (the add-cloud-provider model picker, T068) and the connection-test
/// probe below.
///
/// Key discipline (F_F1 §4.2): the transient key is wrapped in a [`Secret`],
/// injected into the `Authorization` header for this one request, and dropped
/// (zeroized) before the frame returns. Error payloads carry only endpoint
/// kind / HTTP status — never the key or any response body text (09 §5).
pub async fn list_models(
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<Vec<String>, ProviderError> {
    let base = base_url
        .map(str::to_string)
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
    let url = openai_endpoint(&base, "models");
    let http =
        http_client().map_err(|_| ProviderError::Unreachable("http client init failed".into()))?;

    let mut request = http.get(&url);
    if let Some(k) = api_key {
        // Wrapped in `Secret` so the plaintext zeroizes when this block ends.
        let secret = Secret::new(k);
        request = request.header(AUTHORIZATION, format!("Bearer {}", secret.expose()));
    }
    let resp = request.send().await.map_err(map_transport_err)?;

    let resp = ensure_success(resp).await?;
    let parsed: WireModelsList = resp
        .json()
        .await
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

/// One-shot reachability + auth probe behind `verify_ai_provider` (02 §Module H).
///
/// Cross-adapter convention (T059/T060/T062): every cloud/local adapter module
/// exposes this exact signature and the command layer dispatches on
/// `AiProvider`.
///
/// Verification reads the model catalog (`GET /v1/models`) instead of issuing a
/// chat completion. That one change exercises DNS, TLS, and auth end to end
/// while sidestepping the chat-only parameter incompatibilities of current
/// reasoning models (e.g. `gpt-5.5`, which rejects `max_tokens`/`temperature`
/// and requires `max_completion_tokens`) — so picking a real model from the
/// catalog always verifies. Membership in the catalog confirms the model id.
///
/// If the endpoint answers but exposes no usable catalog (404/parse — common on
/// minimal OpenAI-compatible gateways), the probe falls back to the legacy
/// one-shot chat health check so those gateways still verify. Conclusive
/// outcomes (`Auth`, `RateLimited`, `Unreachable`) surface immediately.
pub async fn probe(
    model: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<ProviderHealth, ProviderError> {
    let started = Instant::now();
    match list_models(api_key, base_url).await {
        Ok(models) => {
            let latency_ms = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
            if models.iter().any(|m| m == model) {
                Ok(ProviderHealth {
                    ok: true,
                    model_name: Some(model.to_string()),
                    latency_ms,
                })
            } else {
                // Catalog loaded but does not list this id. Surface a 404-class
                // failure (status only, no model text) so the UI shows the
                // "model not found on this endpoint" copy.
                Err(ProviderError::BadResponse(
                    "http 404 (model not in catalog)".into(),
                ))
            }
        }
        // Conclusive failures — re-probing the chat endpoint cannot change them.
        Err(
            err @ (ProviderError::Auth
            | ProviderError::RateLimited { .. }
            | ProviderError::Unreachable(_)),
        ) => Err(err),
        // Reachable, but no usable `/v1/models` (404/500/parse): fall back to the
        // one-shot chat probe so catalog-less gateways still verify.
        Err(_) => {
            let key_source = match api_key {
                Some(key) => KeySource::Direct(Secret::new(key)),
                None => KeySource::NoAuth,
            };
            let client = OpenAiClient::build(
                model.to_string(),
                key_source,
                base_url.map(str::to_string),
                None,
                Keychain::new(),
            )
            .map_err(|_| ProviderError::BadResponse("http client init failed".into()))?;
            client.health().await
        }
    }
}

#[async_trait]
impl AiProviderClient for OpenAiClient {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, ProviderError> {
        // Identifiers only — no key, no prompt, no completion (09 §5).
        tracing::debug!(
            event = "ai_chat_request",
            provider = "openai",
            model = %req.model,
            request_id = %req.request_id,
            purpose = req.purpose.as_str(),
            "sending chat completion request"
        );

        let started = Instant::now();
        // `send_completions` already adapted the request shape and returns a 2xx.
        let resp = self.send_completions(&req, false).await?;

        let parsed: WireResponse = resp.json().await.map_err(|_| {
            ProviderError::BadResponse("completion body did not parse as json".into())
        })?;
        let latency_ms = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;

        let choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| ProviderError::BadResponse("completion had no choices".into()))?;
        let finish = match choice.finish_reason.as_deref() {
            Some("stop") | None => FinishReason::Stop,
            Some("length") => FinishReason::Length,
            Some("content_filter") => FinishReason::ContentFilter,
            Some(_) => FinishReason::Error,
        };
        let usage = parsed
            .usage
            .map(|u| TokenUsage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
            })
            .unwrap_or_default();

        Ok(ChatResponse {
            text: choice.message.content.unwrap_or_default(),
            finish,
            usage,
            model_echo: parsed.model.unwrap_or_else(|| req.model.clone()),
            latency_ms,
        })
    }

    async fn chat_stream(&self, req: ChatRequest) -> Result<ChatDeltaStream, ProviderError> {
        // Identifiers only — no key, no prompt, no completion (09 §5).
        tracing::debug!(
            event = "ai_chat_stream_request",
            provider = "openai",
            model = %req.model,
            request_id = %req.request_id,
            purpose = req.purpose.as_str(),
            "opening streaming chat completion"
        );

        // `send_completions` adapts the request shape and returns a 2xx; an
        // initial-response failure has already classified like `chat()` (card §3).
        let resp = self.send_completions(&req, true).await?;

        // The response body is moved into the stream state; dropping the
        // returned stream drops it and closes the connection (dev/06 §4).
        let body = resp
            .bytes_stream()
            .map(|chunk| chunk.map(|b| b.to_vec()))
            .boxed();
        Ok(sse::delta_stream(
            body,
            map_transport_err,
            openai_stream_action,
        ))
    }

    async fn health(&self) -> Result<ProviderHealth, ProviderError> {
        // Minimal one-token probe (F_F1 §4.3): cheapest request that exercises
        // DNS, TLS, auth, and the model name end to end.
        let mut req = ChatRequest::simple(self.model.clone(), "hello", Capability::Summarize);
        req.max_tokens = 1;
        req.temperature = 0.0;
        let resp = self.chat(req).await?;
        Ok(ProviderHealth {
            ok: true,
            model_name: Some(resp.model_echo),
            latency_ms: resp.latency_ms,
        })
    }

    fn id(&self) -> AiProvider {
        AiProvider::Openai
    }

    fn context_window(&self) -> usize {
        // Known slug → known window; unknown slugs fall back conservatively to
        // 8192 instead of erroring (dev/06 §5, card §6).
        let model = self.model.as_str();
        if model.starts_with("gpt-4o") || model.starts_with("gpt-4-turbo") {
            128_000
        } else if model.starts_with("gpt-3.5-turbo") {
            16_385
        } else {
            8_192
        }
    }
}

/// Shared HTTP client: connect 10 s, total 60 s (dev/06 §6, F_F1 §4.5).
fn http_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TOTAL_TIMEOUT)
        .build()
}

/// Join an OpenAI-style resource path onto a base URL, tolerating both base
/// conventions seen across vendors (dev/06 §1). When the base already carries a
/// version segment — `…/v1`, `…/v1beta/openai` (Gemini), `…/compatible-mode/v1`
/// (Qwen), `…/api/paas/v4` (Zhipu), `…/v1` (xAI) — the resource is appended
/// directly; for a plain host like `https://api.openai.com` a `/v1` is inserted.
/// `resource` carries no leading slash, e.g. `"chat/completions"` or `"models"`.
fn openai_endpoint(base_url: &str, resource: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base_has_version_segment(base) {
        format!("{base}/{resource}")
    } else {
        format!("{base}/v1/{resource}")
    }
}

/// True when the base URL's path already contains a `v<digit>` segment, so a
/// second `/v1` must not be inserted. Only path segments are examined — the
/// host (and any `v2.example.com`-style host) is skipped.
fn base_has_version_segment(base_url: &str) -> bool {
    let after_scheme = base_url.split("://").nth(1).unwrap_or(base_url);
    let mut segments = after_scheme.split('/');
    segments.next(); // skip host[:port]
    segments.any(|seg| {
        let mut chars = seg.chars();
        chars.next() == Some('v') && chars.next().is_some_and(|c| c.is_ascii_digit())
    })
}

/// Map a reqwest transport failure to [`ProviderError`] with a fixed,
/// content-free tag (09 §5): `reqwest::Error`'s `Display` can embed the URL
/// and other context, so it is reduced to a classification here.
fn map_transport_err(err: reqwest::Error) -> ProviderError {
    if err.is_timeout() {
        ProviderError::Unreachable("request timed out".into())
    } else if err.is_connect() {
        ProviderError::Unreachable("connection failed".into())
    } else {
        ProviderError::Unreachable("transport error".into())
    }
}

// ── OpenAI wire shapes (request) ─────────────────────────────────────────────

#[derive(Serialize)]
struct WireMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct WireChatBody<'a> {
    model: &'a str,
    messages: Vec<WireMessage<'a>>,
    /// Legacy token cap (classic models + most OpenAI-compatible gateways).
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    /// Modern token cap (current reasoning models reject `max_tokens`).
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
    /// Omitted for reasoning models, which accept only the default temperature.
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    stop: &'a [String],
    stream: bool,
}

// ── OpenAI wire shapes (response) ────────────────────────────────────────────

#[derive(Deserialize)]
struct WireResponse {
    #[serde(default)]
    choices: Vec<WireChoice>,
    #[serde(default)]
    usage: Option<WireUsage>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Deserialize)]
struct WireChoice {
    message: WireChoiceMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct WireChoiceMessage {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

// ── OpenAI wire shapes (`GET /v1/models` catalog, T068) ──────────────────────

#[derive(Deserialize)]
struct WireModelsList {
    #[serde(default)]
    data: Vec<WireModelEntry>,
}

#[derive(Deserialize)]
struct WireModelEntry {
    #[serde(default)]
    id: String,
}

// ── OpenAI wire shapes (SSE stream chunks, T061) ─────────────────────────────

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::types::{ChatMessage, ChatRole};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A unique key value used to assert log/error safety: it must never
    /// appear in any error payload.
    const TEST_KEY: &str = "sk-unit-test-key-must-never-leak";

    fn client_for(base_url: &str, api_key: Option<&str>) -> OpenAiClient {
        let key_source = match api_key {
            Some(key) => KeySource::Direct(Secret::new(key)),
            None => KeySource::NoAuth,
        };
        OpenAiClient::build(
            "gpt-4o".into(),
            key_source,
            Some(base_url.to_string()),
            None,
            Keychain::new(),
        )
        .expect("client builds")
    }

    fn success_body() -> serde_json::Value {
        serde_json::json!({
            "id": "chatcmpl-unit",
            "model": "gpt-4o-2024-08-06",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "ok" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 12, "completion_tokens": 1, "total_tokens": 13 }
        })
    }

    fn draft_request() -> ChatRequest {
        let mut req = ChatRequest::simple("gpt-4o", "hello", Capability::DraftReply);
        req.system = "You are a careful assistant.".into();
        req.messages.push(ChatMessage {
            role: ChatRole::Assistant,
            content: "Earlier reply.".into(),
        });
        req
    }

    #[tokio::test]
    async fn openai_chat_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("Authorization", format!("Bearer {TEST_KEY}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(success_body()))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_for(&server.uri(), Some(TEST_KEY));
        let resp = client.chat(draft_request()).await.unwrap();

        assert_eq!(resp.text, "ok");
        assert_eq!(resp.finish, FinishReason::Stop);
        assert_eq!(resp.usage.prompt_tokens, 12);
        assert_eq!(resp.usage.completion_tokens, 1);
        assert_eq!(resp.model_echo, "gpt-4o-2024-08-06");

        // The system preamble must be the FIRST wire message (dev/06 §2.1) and
        // local audit fields must never reach the provider.
        let requests = server.received_requests().await.unwrap();
        let sent: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(sent["messages"][0]["role"], "system");
        assert_eq!(sent["messages"][1]["role"], "user");
        assert_eq!(sent["messages"][2]["role"], "assistant");
        assert!(sent.get("purpose").is_none());
        assert!(sent.get("request_id").is_none());
    }

    #[tokio::test]
    async fn openai_chat_401_returns_auth_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(401)
                    .set_body_string(r#"{"error":{"message":"Incorrect API key provided"}}"#),
            )
            .mount(&server)
            .await;

        let client = client_for(&server.uri(), Some(TEST_KEY));
        let err = client.chat(draft_request()).await.unwrap_err();
        assert_eq!(err, ProviderError::Auth);
    }

    #[tokio::test]
    async fn openai_chat_429_with_retry_after() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "30"))
            .mount(&server)
            .await;

        let client = client_for(&server.uri(), Some(TEST_KEY));
        let err = client.chat(draft_request()).await.unwrap_err();
        assert_eq!(
            err,
            ProviderError::RateLimited {
                retry_after: Some(Duration::from_secs(30)),
            }
        );
    }

    #[tokio::test]
    async fn openai_chat_context_too_long() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(400).set_body_string(
                r#"{"error":{"code":"context_length_exceeded","message":"too many tokens"}}"#,
            ))
            .mount(&server)
            .await;

        let client = client_for(&server.uri(), Some(TEST_KEY));
        let err = client.chat(draft_request()).await.unwrap_err();
        assert_eq!(err, ProviderError::ContextTooLong);
    }

    #[tokio::test]
    async fn openai_connection_refused_is_unreachable() {
        // Port 1 (tcpmux) is closed on dev/CI machines; the connect is refused
        // well inside the 10 s connect timeout.
        let client = client_for("http://127.0.0.1:1", Some(TEST_KEY));
        let err = client.chat(draft_request()).await.unwrap_err();
        assert!(
            matches!(err, ProviderError::Unreachable(_)),
            "expected Unreachable, got {err:?}"
        );
    }

    #[tokio::test]
    async fn openai_health_ok() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(success_body()))
            .mount(&server)
            .await;

        let client = client_for(&server.uri(), Some(TEST_KEY));
        let health = client.health().await.unwrap();
        assert!(health.ok);
        assert_eq!(health.model_name.as_deref(), Some("gpt-4o-2024-08-06"));

        // The probe must be the minimal request (max_tokens = 1, F_F1 §4.3).
        let requests = server.received_requests().await.unwrap();
        let sent: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(sent["max_tokens"], 1);
        assert_eq!(sent["messages"][0]["content"], "hello");
    }

    #[tokio::test]
    async fn probe_succeeds_with_direct_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("Authorization", format!("Bearer {TEST_KEY}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(success_body()))
            .mount(&server)
            .await;

        let health = probe("gpt-4o", Some(TEST_KEY), Some(&server.uri()))
            .await
            .unwrap();
        assert!(health.ok);
        assert_eq!(health.model_name.as_deref(), Some("gpt-4o-2024-08-06"));
    }

    // ── model catalog + catalog-based probe (T068) ──────────────────────────

    fn models_body(ids: &[&str]) -> serde_json::Value {
        let data: Vec<serde_json::Value> = ids
            .iter()
            .map(|id| serde_json::json!({ "id": id, "object": "model" }))
            .collect();
        serde_json::json!({ "object": "list", "data": data })
    }

    #[tokio::test]
    async fn list_models_parses_catalog_sorted_and_deduped() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("Authorization", format!("Bearer {TEST_KEY}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(models_body(&["gpt-5.5", "gpt-4o", "gpt-5.4", "gpt-5.5"])),
            )
            .mount(&server)
            .await;

        let models = list_models(Some(TEST_KEY), Some(&server.uri()))
            .await
            .unwrap();
        assert_eq!(models, vec!["gpt-4o", "gpt-5.4", "gpt-5.5"]);
    }

    #[tokio::test]
    async fn list_models_401_is_auth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let err = list_models(Some(TEST_KEY), Some(&server.uri()))
            .await
            .unwrap_err();
        assert_eq!(err, ProviderError::Auth);
    }

    #[tokio::test]
    async fn list_models_500_never_leaks_key() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(500).set_body_string("server boom"))
            .mount(&server)
            .await;
        let err = list_models(Some(TEST_KEY), Some(&server.uri()))
            .await
            .unwrap_err();
        let rendered = format!("{err} {err:?}");
        assert!(!rendered.contains("Bearer"), "leaked header: {rendered}");
        assert!(!rendered.contains("sk-"), "leaked key prefix: {rendered}");
    }

    /// A current reasoning model (gpt-5.5) verifies by catalog membership — no
    /// chat completion is issued, so its `max_tokens`/`temperature` rejection
    /// can never fail the connection test.
    #[tokio::test]
    async fn probe_ok_when_model_in_catalog() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(models_body(&["gpt-5.5", "gpt-5.4"])),
            )
            .mount(&server)
            .await;

        let health = probe("gpt-5.5", Some(TEST_KEY), Some(&server.uri()))
            .await
            .unwrap();
        assert!(health.ok);
        assert_eq!(health.model_name.as_deref(), Some("gpt-5.5"));
    }

    #[tokio::test]
    async fn probe_model_not_in_catalog_renders_404() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(models_body(&["gpt-5.4"])))
            .mount(&server)
            .await;
        let err = probe("gpt-5.5", Some(TEST_KEY), Some(&server.uri()))
            .await
            .unwrap_err();
        // The UI maps any message containing "404" to "model not found".
        assert!(format!("{err}").contains("404"), "got: {err}");
    }

    /// A minimal OpenAI-compatible gateway with no `/v1/models` route (here:
    /// unmounted → 404) must still verify by falling back to the chat probe.
    #[tokio::test]
    async fn probe_falls_back_to_chat_without_a_catalog() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("Authorization", format!("Bearer {TEST_KEY}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(success_body()))
            .mount(&server)
            .await;
        let health = probe("gpt-4o", Some(TEST_KEY), Some(&server.uri()))
            .await
            .unwrap();
        assert!(health.ok);
        assert_eq!(health.model_name.as_deref(), Some("gpt-4o-2024-08-06"));
    }

    #[tokio::test]
    async fn error_payloads_never_carry_key_or_body_text() {
        const BODY_MARKER: &str = "UNIQUE-RESPONSE-BODY-MARKER-7741";
        let server = MockServer::start().await;
        // A 400 whose body is NOT a context error and a 500 with a marker body:
        // neither body may surface in any error payload (09 §5, card §6).
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_string(format!(r#"{{"error":{{"message":"{BODY_MARKER}"}}}}"#)),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(503).set_body_string(BODY_MARKER))
            .mount(&server)
            .await;

        let client = client_for(&server.uri(), Some(TEST_KEY));
        let bad_request_err = client.chat(draft_request()).await.unwrap_err();
        let server_err = client.chat(draft_request()).await.unwrap_err();

        for err in [&bad_request_err, &server_err] {
            let rendered = format!("{err} {err:?}");
            assert!(!rendered.contains("Bearer"), "leaked header: {rendered}");
            assert!(!rendered.contains("sk-"), "leaked key prefix: {rendered}");
            assert!(!rendered.contains(BODY_MARKER), "leaked body: {rendered}");
        }
        assert_eq!(
            bad_request_err,
            ProviderError::BadResponse("http 400".into())
        );
        assert_eq!(server_err, ProviderError::BadResponse("http 503".into()));
    }

    // ── adaptive request parameters (reasoning models vs legacy gateways) ────

    #[tokio::test]
    async fn reasoning_model_uses_max_completion_tokens_and_omits_temperature() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(success_body()))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_for(&server.uri(), Some(TEST_KEY));
        client
            .chat(ChatRequest::simple(
                "gpt-5.5",
                "hello",
                Capability::DraftReply,
            ))
            .await
            .unwrap();

        let requests = server.received_requests().await.unwrap();
        let sent: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(sent.get("max_completion_tokens").is_some());
        assert!(sent.get("max_tokens").is_none());
        assert!(sent.get("temperature").is_none());
    }

    #[tokio::test]
    async fn legacy_model_uses_max_tokens_and_temperature() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(success_body()))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_for(&server.uri(), Some(TEST_KEY));
        client.chat(draft_request()).await.unwrap(); // gpt-4o

        let requests = server.received_requests().await.unwrap();
        let sent: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(sent.get("max_tokens").is_some());
        assert!(sent.get("max_completion_tokens").is_none());
        assert!(sent.get("temperature").is_some());
    }

    /// Legacy default rejected → must rename to `max_completion_tokens` and retry.
    #[tokio::test]
    async fn adapts_when_endpoint_demands_max_completion_tokens() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(400).set_body_string(
                r#"{"error":{"message":"Unsupported parameter: 'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead.","type":"invalid_request_error","param":"max_tokens"}}"#,
            ))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(success_body()))
            .mount(&server)
            .await;

        let client = client_for(&server.uri(), Some(TEST_KEY));
        let resp = client.chat(draft_request()).await.unwrap();
        assert_eq!(resp.text, "ok");

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2, "must retry once after the parameter 400");
        let first: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        let second: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
        assert!(first.get("max_tokens").is_some());
        assert!(second.get("max_completion_tokens").is_some());
        assert!(second.get("max_tokens").is_none());
    }

    /// A custom temperature rejected by a reasoning model → must drop it and retry.
    #[tokio::test]
    async fn adapts_when_temperature_unsupported() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(400).set_body_string(
                r#"{"error":{"message":"Unsupported value: 'temperature' does not support 0.7 with this model. Only the default (1) value is supported.","type":"invalid_request_error","param":"temperature"}}"#,
            ))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(success_body()))
            .mount(&server)
            .await;

        let client = client_for(&server.uri(), Some(TEST_KEY));
        let resp = client.chat(draft_request()).await.unwrap(); // gpt-4o → sends temperature first
        assert_eq!(resp.text, "ok");

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        let first: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        let second: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
        assert!(first.get("temperature").is_some());
        assert!(second.get("temperature").is_none());
    }

    // ── chat_stream (T061) ──────────────────────────────────────────────────

    fn sse_template(body: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_raw(body.as_bytes().to_vec(), "text/event-stream")
    }

    #[tokio::test]
    async fn openai_stream_three_deltas() {
        let sse_body = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"tok1\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"tok2\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"tok3\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("Authorization", format!("Bearer {TEST_KEY}")))
            .respond_with(sse_template(sse_body))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_for(&server.uri(), Some(TEST_KEY));
        let mut stream = client.chat_stream(draft_request()).await.unwrap();
        let mut deltas = Vec::new();
        while let Some(item) = stream.next().await {
            deltas.push(item.unwrap());
        }

        assert_eq!(deltas.len(), 3);
        for (i, text) in ["tok1", "tok2", "tok3"].iter().enumerate() {
            assert_eq!(deltas[i].text, *text);
            assert_eq!(deltas[i].index, i, "delta ordinals must be 0-based");
        }

        // The streaming request must flag `stream: true` on the wire and keep
        // local audit fields off it (dev/06 §2.1).
        let requests = server.received_requests().await.unwrap();
        let sent: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(sent["stream"], true);
        assert!(sent.get("purpose").is_none());
        assert!(sent.get("request_id").is_none());
    }

    #[tokio::test]
    async fn openai_stream_ignores_data_after_done() {
        let sse_body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"only\"}}]}\n\n",
            "data: [DONE]\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"late\"}}]}\n\n",
        );
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(sse_template(sse_body))
            .mount(&server)
            .await;

        let client = client_for(&server.uri(), Some(TEST_KEY));
        let stream = client.chat_stream(draft_request()).await.unwrap();
        let collected: Vec<_> = stream.collect().await;
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].as_ref().unwrap().text, "only");
    }

    /// Mid-stream malformed JSON yields one `Err(BadResponse)` and terminates;
    /// the malformed payload text never enters the error (09 §5).
    #[tokio::test]
    async fn openai_stream_midstream_malformed_json_fails_without_leak() {
        const PAYLOAD_MARKER: &str = "CONFIDENTIAL-DELTA-PAYLOAD-9152";
        let sse_body = format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"good\"}}}}]}}\n\n\
             data: {{not json {PAYLOAD_MARKER}\n\n\
             data: {{\"choices\":[{{\"delta\":{{\"content\":\"never\"}}}}]}}\n\n"
        );
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(sse_template(&sse_body))
            .mount(&server)
            .await;

        let client = client_for(&server.uri(), Some(TEST_KEY));
        let mut stream = client.chat_stream(draft_request()).await.unwrap();
        assert_eq!(stream.next().await.unwrap().unwrap().text, "good");
        let err = stream.next().await.unwrap().unwrap_err();
        assert!(matches!(err, ProviderError::BadResponse(_)));
        let rendered = format!("{err} {err:?}");
        assert!(
            !rendered.contains(PAYLOAD_MARKER),
            "leaked payload: {rendered}"
        );
        assert!(
            stream.next().await.is_none(),
            "stream must terminate after the error"
        );
    }

    /// An initial-response failure on the streaming path classifies exactly
    /// like a `chat()` failure (card §3).
    #[tokio::test]
    async fn openai_stream_initial_response_errors_classify_like_chat() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(401))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "30"))
            .mount(&server)
            .await;

        let client = client_for(&server.uri(), Some(TEST_KEY));
        let auth_err = client.chat_stream(draft_request()).await.err().unwrap();
        assert_eq!(auth_err, ProviderError::Auth);

        let rate_err = client.chat_stream(draft_request()).await.err().unwrap();
        assert_eq!(
            rate_err,
            ProviderError::RateLimited {
                retry_after: Some(Duration::from_secs(30)),
            }
        );
    }

    /// Cancellation = dropping the stream handle: the HTTP connection closes
    /// and nothing hangs (dev/06 §4). A raw TCP server is used because the
    /// connection must stay open mid-stream to observe the close.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openai_stream_cancel_closes_connection() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::mpsc;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            // Drain whatever part of the request has arrived; the response
            // does not depend on its content.
            let mut buf = [0u8; 8192];
            let _ = socket.read(&mut buf);
            // Chunked SSE response: one delta, then the stream is held open.
            let head = concat!(
                "HTTP/1.1 200 OK\r\n",
                "content-type: text/event-stream\r\n",
                "transfer-encoding: chunked\r\n",
                "\r\n",
            );
            let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"tok\"}}]}\n\n";
            let chunk = format!("{:x}\r\n{sse}\r\n", sse.len());
            socket.write_all(head.as_bytes()).unwrap();
            socket.write_all(chunk.as_bytes()).unwrap();
            socket.flush().unwrap();
            // Wait until the client closes the connection: EOF or error.
            loop {
                match socket.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            let _ = tx.send(());
        });

        let client = client_for(&format!("http://{addr}"), Some(TEST_KEY));
        let mut stream = client.chat_stream(draft_request()).await.unwrap();
        let first = stream.next().await.unwrap().unwrap();
        assert_eq!(first.text, "tok");

        // Cancel by dropping the stream; the partial is discarded.
        drop(stream);

        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("server must observe the connection close after the stream is dropped");
    }

    #[test]
    fn context_window_per_model_slug() {
        let window = |model: &str| {
            OpenAiClient::build(model.into(), KeySource::NoAuth, None, None, Keychain::new())
                .unwrap()
                .context_window()
        };
        assert_eq!(window("gpt-4o"), 128_000);
        assert_eq!(window("gpt-4o-mini"), 128_000);
        assert_eq!(window("gpt-4-turbo"), 128_000);
        assert_eq!(window("gpt-3.5-turbo"), 16_385);
        assert_eq!(window("some-future-model"), 8_192);
    }

    #[test]
    fn from_config_requires_model() {
        let cfg = AccountAiConfig {
            account_id: "00000000-0000-0000-0000-000000000000".into(),
            provider: AiProvider::Openai,
            model: None,
            base_url: None,
            api_key_ref: None,
            daily_query_limit: 10,
            updated_at: 0,
        };
        let err = OpenAiClient::from_config(&cfg, Keychain::new()).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn from_config_builds_with_model_and_key_ref() {
        let cfg = AccountAiConfig {
            account_id: "00000000-0000-0000-0000-000000000000".into(),
            provider: AiProvider::Openai,
            model: Some("gpt-4o".into()),
            base_url: Some("https://gateway.example.com".into()),
            api_key_ref: Some("00000000-0000-0000-0000-000000000000".into()),
            daily_query_limit: 10,
            updated_at: 0,
        };
        let client = OpenAiClient::from_config(&cfg, Keychain::new()).unwrap();
        assert_eq!(client.id(), AiProvider::Openai);
        assert_eq!(
            client.completions_url(),
            "https://gateway.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn endpoint_url_tolerates_versioned_and_plain_bases() {
        // Plain host → insert /v1 (OpenAI, DeepSeek, Mistral host root, …).
        assert_eq!(
            openai_endpoint("https://api.openai.com", "chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            openai_endpoint("https://api.deepseek.com", "models"),
            "https://api.deepseek.com/v1/models"
        );
        // Already-versioned bases → append directly (never a doubled /v1).
        assert_eq!(
            openai_endpoint("https://api.x.ai/v1", "chat/completions"),
            "https://api.x.ai/v1/chat/completions"
        );
        assert_eq!(
            openai_endpoint(
                "https://generativelanguage.googleapis.com/v1beta/openai",
                "chat/completions"
            ),
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
        );
        assert_eq!(
            openai_endpoint(
                "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
                "models"
            ),
            "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/models"
        );
        assert_eq!(
            openai_endpoint("https://open.bigmodel.cn/api/paas/v4", "chat/completions"),
            "https://open.bigmodel.cn/api/paas/v4/chat/completions"
        );
        // Trailing slash tolerated.
        assert_eq!(
            openai_endpoint("https://api.x.ai/v1/", "models"),
            "https://api.x.ai/v1/models"
        );
    }
}
