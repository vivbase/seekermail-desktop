//! Neutral request/response types for the BYO AI subsystem (T058, dev/06 §2.1).
//!
//! Every concrete adapter (OpenAI, Anthropic, Ollama, local ONNX) maps these
//! provider-agnostic shapes to/from its vendor wire format. The rest of the
//! system — draft engine, risk engine, role analyzers — only ever sees these
//! types, never a vendor SDK shape.

use serde::{Deserialize, Serialize};
use specta::Type;
use uuid::Uuid;

pub use crate::types::AiProvider;

/// What a generation/analysis call is *for*. Drives F4 capability×account
/// routing and the `ai_decisions` audit trail; never sent to the provider
/// (dev/06 §2.1).
///
/// Wire/JSON representation (T065 §6): the PascalCase variant name
/// (`"DraftReply"`, `"RiskReason"`, …) — this is what the persisted F4 matrix
/// and the specta-generated TypeScript union carry. [`Capability::as_str`]
/// stays snake_case for logs and `ai_decisions` rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
pub enum Capability {
    DraftReply,
    RiskReason,
    Summarize,
    StyleProfile,
}

impl Capability {
    /// Stable tag for logs and `ai_decisions` rows (identifiers only, 09 §5).
    pub fn as_str(self) -> &'static str {
        match self {
            Capability::DraftReply => "draft_reply",
            Capability::RiskReason => "risk_reason",
            Capability::Summarize => "summarize",
            Capability::StyleProfile => "style_profile",
        }
    }
}

/// One user/assistant turn inside a [`ChatRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

/// Conversation role for a [`ChatMessage`]. The system preamble travels in
/// `ChatRequest::system`, not as a message, so adapters can map it to whatever
/// the vendor expects (top-level field vs. leading message).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
}

impl ChatRole {
    pub fn as_str(self) -> &'static str {
        match self {
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
        }
    }
}

/// Provider-agnostic completion request (dev/06 §2.1).
#[derive(Debug, Clone)]
pub struct ChatRequest {
    /// Resolved from `account_ai_settings.ai_model`.
    pub model: String,
    /// Role + safety preamble (dev/06 §5). Assembled by the context packer.
    pub system: String,
    /// User/assistant turns, oldest first.
    pub messages: Vec<ChatMessage>,
    pub max_tokens: u32,
    /// Drafts ~0.3; risk reasoning 0.0 (dev/06 §2.1).
    pub temperature: f32,
    pub stop: Vec<String>,
    /// For audit + routing — never serialized onto the provider wire.
    pub purpose: Capability,
    /// Local correlation id for the `ai_decisions` audit log.
    pub request_id: Uuid,
}

impl ChatRequest {
    /// A minimal well-formed request, used by tests and health probes.
    pub fn simple(
        model: impl Into<String>,
        user_text: impl Into<String>,
        purpose: Capability,
    ) -> Self {
        Self {
            model: model.into(),
            system: String::new(),
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: user_text.into(),
            }],
            max_tokens: 1024,
            temperature: 0.3,
            stop: Vec::new(),
            purpose,
            request_id: Uuid::new_v4(),
        }
    }
}

/// Why the provider stopped generating (dev/06 §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    Length,
    ContentFilter,
    Error,
}

/// Best-effort token accounting echoed by the provider (dev/06 §2.1). Backs the
/// `input_tokens`/`output_tokens` columns of `ai_decisions`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

/// Provider-agnostic completion response (dev/06 §2.1).
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub text: String,
    pub finish: FinishReason,
    pub usage: TokenUsage,
    /// The model the provider actually used (may differ from the request).
    pub model_echo: String,
    pub latency_ms: u32,
}

/// One streamed token chunk (dev/06 §4). `index` is the 0-based delta ordinal
/// so the frontend can detect gaps after a reconnect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatDelta {
    pub text: String,
    pub index: usize,
}

/// Result of the lightweight reachability + auth probe behind
/// `verify_ai_provider` (dev/06 §2, 02 §Module H).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHealth {
    pub ok: bool,
    /// Confirmed model name from the provider, when it reports one.
    pub model_name: Option<String>,
    pub latency_ms: u32,
}
