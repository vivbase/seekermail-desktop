//! `MockProvider` — the scripted test seam for the BYO AI subsystem (T058,
//! dev/06 §10). Compiled only for tests.
//!
//! Adapters, the degradation matrix (T061/T067), and the draft/risk engines
//! all exercise their AI paths against this mock with zero network and zero
//! spend: push responses/errors in call order, inject stream deltas, and flip
//! health between ok and fail.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use futures::stream;

use crate::types::AiProvider;

use super::provider::{AiProviderClient, ChatDeltaStream, ProviderError};
use super::types::{
    ChatDelta, ChatRequest, ChatResponse, FinishReason, ProviderHealth, TokenUsage,
};

/// Scripted in-memory provider. All queues are interior-mutable so tests can
/// keep pushing after the mock is shared as `Arc<dyn AiProviderClient>`.
pub struct MockProvider {
    id: AiProvider,
    context_window: usize,
    chat_script: Mutex<VecDeque<Result<ChatResponse, ProviderError>>>,
    stream_script: Mutex<VecDeque<Result<Vec<ChatDelta>, ProviderError>>>,
    health_script: Mutex<VecDeque<Result<ProviderHealth, ProviderError>>>,
    /// When set, `chat()` returns this error whenever the script queue is
    /// empty — a persistently-down provider for degradation tests (T067).
    default_chat_error: Mutex<Option<ProviderError>>,
    /// Calls observed, for assertions on retry behavior.
    chat_calls: Mutex<u32>,
    /// Health probes observed, for assertions on probe timing (T067).
    health_calls: Mutex<u32>,
}

impl MockProvider {
    /// A mock that answers every call successfully with a canned response.
    pub fn healthy(id: AiProvider) -> Self {
        Self {
            id,
            context_window: 8_192,
            chat_script: Mutex::new(VecDeque::new()),
            stream_script: Mutex::new(VecDeque::new()),
            health_script: Mutex::new(VecDeque::new()),
            default_chat_error: Mutex::new(None),
            chat_calls: Mutex::new(0),
            health_calls: Mutex::new(0),
        }
    }

    pub fn with_context_window(mut self, tokens: usize) -> Self {
        self.context_window = tokens;
        self
    }

    /// Queue the next `chat()` outcome (consumed in FIFO order).
    pub fn push_chat(&self, outcome: Result<ChatResponse, ProviderError>) {
        self.chat_script.lock().unwrap().push_back(outcome);
    }

    /// Queue the next `chat_stream()` outcome: a full delta sequence, or an
    /// error raised before the stream opens.
    pub fn inject_stream_deltas(&self, deltas: Vec<ChatDelta>) {
        self.stream_script.lock().unwrap().push_back(Ok(deltas));
    }

    pub fn push_stream_error(&self, err: ProviderError) {
        self.stream_script.lock().unwrap().push_back(Err(err));
    }

    /// Queue the next `health()` outcome.
    pub fn push_health(&self, outcome: Result<ProviderHealth, ProviderError>) {
        self.health_script.lock().unwrap().push_back(outcome);
    }

    /// Make every unscripted `chat()` fail with `err` — a provider that stays
    /// down for an arbitrary number of calls (T067 degradation sequences).
    /// Scripted `push_chat` outcomes still take precedence in FIFO order.
    pub fn set_default_chat_error(&self, err: ProviderError) {
        *self.default_chat_error.lock().unwrap() = Some(err);
    }

    /// Restore the canned success default — the provider "recovered".
    pub fn clear_default_chat_error(&self) {
        *self.default_chat_error.lock().unwrap() = None;
    }

    /// How many times `chat()` was invoked (for retry assertions).
    pub fn chat_call_count(&self) -> u32 {
        *self.chat_calls.lock().unwrap()
    }

    /// How many times `health()` was invoked (for probe-timing assertions).
    pub fn health_call_count(&self) -> u32 {
        *self.health_calls.lock().unwrap()
    }

    /// The canned success returned when the script queue is empty.
    fn default_response(&self) -> ChatResponse {
        ChatResponse {
            text: "scripted mock completion".into(),
            finish: FinishReason::Stop,
            usage: TokenUsage {
                prompt_tokens: 12,
                completion_tokens: 5,
            },
            model_echo: "mock-model".into(),
            latency_ms: 1,
        }
    }
}

#[async_trait]
impl AiProviderClient for MockProvider {
    async fn chat(&self, _req: ChatRequest) -> Result<ChatResponse, ProviderError> {
        *self.chat_calls.lock().unwrap() += 1;
        match self.chat_script.lock().unwrap().pop_front() {
            Some(outcome) => outcome,
            None => match self.default_chat_error.lock().unwrap().clone() {
                Some(err) => Err(err),
                None => Ok(self.default_response()),
            },
        }
    }

    async fn chat_stream(&self, _req: ChatRequest) -> Result<ChatDeltaStream, ProviderError> {
        match self.stream_script.lock().unwrap().pop_front() {
            Some(Ok(deltas)) => {
                let items: Vec<Result<ChatDelta, ProviderError>> =
                    deltas.into_iter().map(Ok).collect();
                Ok(Box::pin(stream::iter(items)))
            }
            Some(Err(e)) => Err(e),
            None => {
                let text = self.default_response().text;
                Ok(Box::pin(stream::iter(vec![Ok(ChatDelta {
                    text,
                    index: 0,
                })])))
            }
        }
    }

    async fn health(&self) -> Result<ProviderHealth, ProviderError> {
        *self.health_calls.lock().unwrap() += 1;
        match self.health_script.lock().unwrap().pop_front() {
            Some(outcome) => outcome,
            None => Ok(ProviderHealth {
                ok: true,
                model_name: Some("mock-model".into()),
                latency_ms: 1,
            }),
        }
    }

    fn id(&self) -> AiProvider {
        self.id
    }

    fn context_window(&self) -> usize {
        self.context_window
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{AppError, IpcError};
    use crate::types::ErrorCode;
    use futures::StreamExt;

    #[tokio::test]
    async fn scripted_ok_then_default() {
        let mock = MockProvider::healthy(AiProvider::Ollama);
        mock.push_chat(Ok(ChatResponse {
            text: "first".into(),
            finish: FinishReason::Stop,
            usage: TokenUsage::default(),
            model_echo: "m".into(),
            latency_ms: 2,
        }));

        let req = ChatRequest::simple("m", "hi", super::super::types::Capability::Summarize);
        let first = mock.chat(req.clone()).await.unwrap();
        assert_eq!(first.text, "first");
        // Queue exhausted → canned default keeps later calls deterministic.
        let second = mock.chat(req).await.unwrap();
        assert_eq!(second.text, "scripted mock completion");
        assert_eq!(mock.chat_call_count(), 2);
    }

    #[tokio::test]
    async fn injected_unreachable_surfaces_wire_code() {
        let mock = MockProvider::healthy(AiProvider::Openai);
        mock.push_chat(Err(ProviderError::Unreachable("refused".into())));

        let req = ChatRequest::simple("m", "hi", super::super::types::Capability::DraftReply);
        let err = mock.chat(req).await.unwrap_err();
        let app: AppError = err.into();
        assert_eq!(app.code().as_wire(), "AI_PROVIDER_UNREACHABLE");
    }

    #[tokio::test]
    async fn injected_stream_deltas_replay_in_order() {
        let mock = MockProvider::healthy(AiProvider::Anthropic);
        mock.inject_stream_deltas(vec![
            ChatDelta {
                text: "Hel".into(),
                index: 0,
            },
            ChatDelta {
                text: "lo".into(),
                index: 1,
            },
        ]);

        let req = ChatRequest::simple("m", "hi", super::super::types::Capability::DraftReply);
        let mut stream = mock.chat_stream(req).await.unwrap();
        let mut collected = String::new();
        while let Some(delta) = stream.next().await {
            collected.push_str(&delta.unwrap().text);
        }
        assert_eq!(collected, "Hello");
    }

    #[tokio::test]
    async fn default_chat_error_persists_until_cleared() {
        let mock = MockProvider::healthy(AiProvider::Openai);
        mock.set_default_chat_error(ProviderError::Unreachable("link down".into()));

        let req = ChatRequest::simple("m", "hi", super::super::types::Capability::DraftReply);
        // Stays down across any number of unscripted calls…
        assert!(mock.chat(req.clone()).await.is_err());
        assert!(mock.chat(req.clone()).await.is_err());
        // …while scripted outcomes still win in FIFO order…
        mock.push_chat(Ok(mock.default_response()));
        assert!(mock.chat(req.clone()).await.is_ok());
        assert!(mock.chat(req.clone()).await.is_err());
        // …and clearing it restores the canned success.
        mock.clear_default_chat_error();
        assert!(mock.chat(req).await.is_ok());
        assert_eq!(mock.chat_call_count(), 5);
    }

    #[tokio::test]
    async fn health_calls_are_counted() {
        let mock = MockProvider::healthy(AiProvider::Ollama);
        assert_eq!(mock.health_call_count(), 0);
        mock.health().await.unwrap();
        mock.health().await.unwrap();
        assert_eq!(mock.health_call_count(), 2);
    }

    #[tokio::test]
    async fn health_failure_is_injectable() {
        let mock = MockProvider::healthy(AiProvider::Ollama);
        mock.push_health(Err(ProviderError::Unreachable("daemon down".into())));
        assert!(mock.health().await.is_err());
        // Next call falls back to healthy.
        assert!(mock.health().await.unwrap().ok);
    }

    /// 09 §5 / dev/06 log-safety: a content-bearing `BadResponse` payload must
    /// never reach the boundary log or the wire `detail`. The boundary log line
    /// is built from the same `detail()` string asserted here, so detail-free
    /// implies log-free.
    #[tokio::test]
    async fn bad_response_payload_never_reaches_wire_or_log() {
        const SENSITIVE: &str = "Dear Dr. Mira Holt, the settlement amount is $4,200,000";
        let mock = MockProvider::healthy(AiProvider::Openai);
        mock.push_chat(Err(ProviderError::BadResponse(SENSITIVE.into())));

        let req = ChatRequest::simple("m", "hi", super::super::types::Capability::RiskReason);
        let provider_err = mock.chat(req).await.unwrap_err();
        let ipc: IpcError = AppError::from(provider_err).into();

        assert_eq!(ipc.code, ErrorCode::Internal);
        assert!(!ipc.message.contains(SENSITIVE));
        assert!(!ipc.detail.unwrap_or_default().contains("Mira Holt"));
    }
}
