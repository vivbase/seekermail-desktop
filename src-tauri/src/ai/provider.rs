//! The `AiProviderClient` trait and provider error model (T058, dev/06 §2, §6).
//!
//! Concrete adapters (T059 OpenAI, T060 Anthropic, T062 Ollama, T063 local
//! ONNX) implement this trait; everything above the trait — registry, draft
//! engine, risk engine — is adapter-agnostic.
//!
//! ADR-0004 (no proxy) applies to every implementor: requests go directly from
//! this process to the endpoint the *user* configured. Nothing in this module
//! knows about, or may ever be given, a SeekerMail server address.

use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use rand::Rng;

use crate::error::AppError;
use crate::types::AiProvider;

use super::types::{Capability, ChatDelta, ChatRequest, ChatResponse, ProviderHealth};

/// Stream alias returned by [`AiProviderClient::chat_stream`].
pub type ChatDeltaStream = BoxStream<'static, Result<ChatDelta, ProviderError>>;

/// One AI backend the user brought (dev/06 §2). Object-safe (`async_trait`) so
/// the registry can hold `Arc<dyn AiProviderClient>` values.
#[async_trait]
pub trait AiProviderClient: Send + Sync {
    /// One-shot completion (drafts, risk reasoning, summaries).
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, ProviderError>;

    /// Streaming completion; yields token deltas for live draft rendering.
    async fn chat_stream(&self, req: ChatRequest) -> Result<ChatDeltaStream, ProviderError>;

    /// Lightweight reachability + auth probe; backs `verify_ai_provider`.
    async fn health(&self) -> Result<ProviderHealth, ProviderError>;

    /// Which provider this client is (registry key).
    fn id(&self) -> AiProvider;

    /// Model token budget, used by the context packer (dev/06 §5).
    fn context_window(&self) -> usize;
}

impl std::fmt::Debug for dyn AiProviderClient {
    /// Identifier-only `Debug` so trait objects can appear in test assertions
    /// and `Result::unwrap_err` panics without exposing adapter internals.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AiProviderClient({})", self.id().as_str())
    }
}

/// Adapter-level failure (dev/06 §6). String payloads carry *technical* detail
/// only — endpoint kind, HTTP status, parse position — never prompt text,
/// completion text, or mail content (09 §5). Adapters are responsible for
/// keeping content out of these payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// Network/DNS/connection-refused.
    Unreachable(String),
    /// 401/403 — the key or endpoint auth is wrong; settings must be re-entered.
    Auth,
    /// 429 — honor `retry_after` when the provider sent one.
    RateLimited { retry_after: Option<Duration> },
    /// The packed context exceeds the model window (HTTP 400 context class).
    ContextTooLong,
    /// Unparseable or empty response body.
    BadResponse(String),
    /// The provider refused to complete (its own content policy).
    ContentFiltered,
    /// Locally aborted (user cancelled the stream).
    Canceled,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::Unreachable(d) => write!(f, "provider unreachable: {d}"),
            ProviderError::Auth => write!(f, "provider auth rejected"),
            ProviderError::RateLimited { retry_after } => match retry_after {
                Some(d) => write!(f, "provider rate limited (retry after {}s)", d.as_secs()),
                None => write!(f, "provider rate limited"),
            },
            ProviderError::ContextTooLong => write!(f, "context too long for model"),
            ProviderError::BadResponse(d) => write!(f, "bad provider response: {d}"),
            ProviderError::ContentFiltered => write!(f, "provider filtered the content"),
            ProviderError::Canceled => write!(f, "request canceled"),
        }
    }
}

impl std::error::Error for ProviderError {}

// ── Retry policy (dev/06 §6, T061) ───────────────────────────────────────────
//
// Lives at the wrapper layer — engines call these helpers instead of the raw
// trait methods — never inside the HTTP adapters, so a `RiskReason` call can
// never be retried by accident (risk reasoning must stay atomic, dev/06 §4).
// Retrying a generation can never double-send mail: sending is a separate,
// explicit, human-gated step after the draft exists (dev/06 §6).

/// Upper bound of the random pre-retry delay (card §3: jitter 0–500 ms).
const RETRY_JITTER_MAX_MS: u64 = 500;

/// A random 0–500 ms pause so simultaneous retries don't stampede a provider
/// that just came back.
fn retry_jitter() -> Duration {
    Duration::from_millis(rand::thread_rng().gen_range(0..=RETRY_JITTER_MAX_MS))
}

/// `chat()` with the dev/06 §6 single-retry policy.
///
/// Retries **exactly once**, and only when *both* hold:
/// * `req.purpose == Capability::DraftReply` (drafts are safe to regenerate —
///   nothing is sent until a human approves), and
/// * the first attempt failed with [`ProviderError::Unreachable`] (the one
///   transient bucket; `RateLimited`, `Auth`, `ContextTooLong`, and the rest
///   are deterministic or externally throttled and must surface immediately).
///
/// The second outcome is returned as-is — there is never a third attempt.
pub async fn chat_with_retry(
    client: &dyn AiProviderClient,
    req: ChatRequest,
) -> Result<ChatResponse, ProviderError> {
    let first = client.chat(req.clone()).await;
    let transient = matches!(first, Err(ProviderError::Unreachable(_)));
    if transient && req.purpose == Capability::DraftReply {
        tracing::debug!(
            event = "ai_chat_retry",
            request_id = %req.request_id,
            purpose = req.purpose.as_str(),
            "draft generation unreachable on first attempt; retrying once after jitter"
        );
        tokio::time::sleep(retry_jitter()).await;
        return client.chat(req).await;
    }
    first
}

/// `health()` with the dev/06 §6 idempotent-read retry: at most **2 attempts**
/// total, the second after a 0–500 ms jitter, and only when the first failed
/// with [`ProviderError::Unreachable`]. Deterministic failures (`Auth`,
/// `BadResponse`, …) surface immediately — re-probing cannot fix them.
pub async fn health_with_retry(
    client: &dyn AiProviderClient,
) -> Result<ProviderHealth, ProviderError> {
    let first = client.health().await;
    if matches!(first, Err(ProviderError::Unreachable(_))) {
        tokio::time::sleep(retry_jitter()).await;
        return client.health().await;
    }
    first
}

impl From<ProviderError> for AppError {
    /// The single `ProviderError → AppError` mapping (dev/06 §6, 09 §2).
    fn from(err: ProviderError) -> Self {
        match err {
            ProviderError::Unreachable(d) => AppError::AiUnreachable(d),
            // Bad key/endpoint is user-correctable: same bucket as bad mail
            // credentials — re-enter settings (09 §2).
            ProviderError::Auth => AppError::AuthInvalidCredentials,
            ProviderError::RateLimited { .. } => AppError::AiRateLimited,
            ProviderError::ContextTooLong => AppError::AiContextTooLong,
            // The payload is deliberately reduced to its length: even though
            // adapters must never place content in `BadResponse`, the boundary
            // conversion stays safe against a buggy adapter — nothing of the
            // payload can reach the log line or the IpcError detail (09 §5).
            ProviderError::BadResponse(d) => AppError::Internal(anyhow::anyhow!(
                "ai provider returned an unparseable response ({} chars)",
                d.len()
            )),
            ProviderError::ContentFiltered => {
                AppError::Forbidden("the AI provider declined to generate this content".into())
            }
            ProviderError::Canceled => {
                AppError::Internal(anyhow::anyhow!("ai request canceled by caller"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ErrorCode;

    #[test]
    fn unreachable_maps_to_ai_provider_unreachable() {
        let app: AppError = ProviderError::Unreachable("connect refused".into()).into();
        assert_eq!(app.code(), ErrorCode::AiProviderUnreachable);
    }

    #[test]
    fn rate_limited_maps_with_and_without_retry_after() {
        let with: AppError = ProviderError::RateLimited {
            retry_after: Some(Duration::from_secs(30)),
        }
        .into();
        let without: AppError = ProviderError::RateLimited { retry_after: None }.into();
        assert_eq!(with.code(), ErrorCode::AiRateLimited);
        assert_eq!(without.code(), ErrorCode::AiRateLimited);
    }

    #[test]
    fn context_too_long_maps_to_user_correctable_code() {
        let app: AppError = ProviderError::ContextTooLong.into();
        assert_eq!(app.code(), ErrorCode::AiContextTooLong);
    }

    #[test]
    fn auth_maps_to_invalid_credentials() {
        let app: AppError = ProviderError::Auth.into();
        assert_eq!(app.code(), ErrorCode::AuthInvalidCredentials);
    }

    // ── Retry policy (dev/06 §6, T061 card §8) ──────────────────────────────
    //
    // `start_paused` virtual time makes the 0–500 ms jitter sleeps instant —
    // MockProvider does no real I/O, so auto-advance is safe here.

    use crate::ai::mock::MockProvider;

    fn request_for(purpose: Capability) -> ChatRequest {
        ChatRequest::simple("mock-model", "draft a reply to the latest mail", purpose)
    }

    #[tokio::test(start_paused = true)]
    async fn chat_draft_retry_on_unreachable() {
        let mock = MockProvider::healthy(AiProvider::Openai);
        mock.push_chat(Err(ProviderError::Unreachable("connect refused".into())));
        // Second attempt falls through to the mock's canned success.

        let resp = chat_with_retry(&mock, request_for(Capability::DraftReply))
            .await
            .unwrap();
        assert_eq!(resp.text, "scripted mock completion");
        assert_eq!(mock.chat_call_count(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn chat_draft_retries_at_most_once() {
        let mock = MockProvider::healthy(AiProvider::Openai);
        mock.push_chat(Err(ProviderError::Unreachable("connect refused".into())));
        mock.push_chat(Err(ProviderError::Unreachable("still down".into())));

        let err = chat_with_retry(&mock, request_for(Capability::DraftReply))
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Unreachable(_)));
        assert_eq!(mock.chat_call_count(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn chat_risk_no_retry_on_unreachable() {
        let mock = MockProvider::healthy(AiProvider::Openai);
        mock.push_chat(Err(ProviderError::Unreachable("connect refused".into())));

        let err = chat_with_retry(&mock, request_for(Capability::RiskReason))
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Unreachable(_)));
        // Risk reasoning must stay atomic: exactly one attempt (dev/06 §4).
        assert_eq!(mock.chat_call_count(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn chat_draft_no_retry_on_non_transient_errors() {
        for err in [
            ProviderError::RateLimited {
                retry_after: Some(Duration::from_secs(30)),
            },
            ProviderError::Auth,
            ProviderError::ContextTooLong,
            ProviderError::BadResponse("http 503".into()),
            ProviderError::ContentFiltered,
            ProviderError::Canceled,
        ] {
            let mock = MockProvider::healthy(AiProvider::Openai);
            mock.push_chat(Err(err.clone()));

            let got = chat_with_retry(&mock, request_for(Capability::DraftReply))
                .await
                .unwrap_err();
            assert_eq!(got, err);
            assert_eq!(mock.chat_call_count(), 1, "no retry allowed for {err:?}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn health_retries_once_on_unreachable_with_jitter() {
        let mock = MockProvider::healthy(AiProvider::Anthropic);
        mock.push_health(Err(ProviderError::Unreachable("connect refused".into())));
        // Second probe falls through to the mock's canned healthy answer.

        let health = health_with_retry(&mock).await.unwrap();
        assert!(health.ok);
    }

    #[tokio::test(start_paused = true)]
    async fn health_does_not_retry_on_auth() {
        let mock = MockProvider::healthy(AiProvider::Anthropic);
        mock.push_health(Err(ProviderError::Auth));
        // If a second probe ran, it would succeed — so an `Err` here proves
        // the helper stopped after one attempt.
        let err = health_with_retry(&mock).await.unwrap_err();
        assert_eq!(err, ProviderError::Auth);
    }

    // ── Degradation matrix (dev/09 §8, card §8) ─────────────────────────────

    #[tokio::test]
    async fn degradation_rate_limited() {
        let mock = MockProvider::healthy(AiProvider::Openai);
        mock.push_chat(Err(ProviderError::RateLimited {
            retry_after: Some(Duration::from_secs(30)),
        }));
        let err = mock
            .chat(request_for(Capability::DraftReply))
            .await
            .unwrap_err();
        let app: AppError = err.into();
        assert_eq!(app.code(), ErrorCode::AiRateLimited);
    }

    #[tokio::test]
    async fn degradation_context_too_long() {
        let mock = MockProvider::healthy(AiProvider::Openai);
        mock.push_chat(Err(ProviderError::ContextTooLong));
        let err = mock
            .chat(request_for(Capability::DraftReply))
            .await
            .unwrap_err();
        let app: AppError = err.into();
        assert_eq!(app.code(), ErrorCode::AiContextTooLong);
    }
}
