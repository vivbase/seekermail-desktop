//! BYO AI subsystem (Module F) — abstraction layer and routing core (T058).
//!
//! Architecture (dev/06, ADR-0004):
//!
//! * [`provider::AiProviderClient`] — the trait every adapter implements.
//! * [`types`] — neutral request/response shapes; nothing vendor-specific
//!   crosses this module's boundary.
//! * [`registry::AiRegistry`] — capability × account routing + the
//!   `daily_query_limit` cost guardrail.
//! * [`mock::MockProvider`] *(tests only)* — the scripted seam every AI test
//!   runs against: zero network, zero spend.
//!
//! **ADR-0004 red-line:** requests go directly from the user's device to the
//! provider endpoint the user configured. No type in this module holds or
//! accepts a SeekerMail server address.

pub mod audit;
pub mod context;
pub mod draft;
pub mod fallback;
pub mod legal;
pub mod matrix;
pub mod mce;
pub mod memory;
pub mod pipeline;
pub mod provider;
pub mod providers;
pub mod qa_card;
pub mod query_detection;
pub mod recommended;
pub mod registry;
pub mod sales;
pub mod settings;
pub mod sse;
pub mod style;
pub mod team_chat;
pub mod types;

#[cfg(test)]
pub mod mock;

pub use provider::{AiProviderClient, ChatDeltaStream, ProviderError};
pub use registry::{AccountAiConfig, AiRegistry, ProviderFactory};
pub use types::{
    AiProvider, Capability, ChatDelta, ChatMessage, ChatRequest, ChatResponse, ChatRole,
    FinishReason, ProviderHealth, TokenUsage,
};
