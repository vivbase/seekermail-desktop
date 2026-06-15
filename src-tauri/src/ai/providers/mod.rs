//! Concrete `AiProviderClient` adapters (Module F).
//!
//! One file per provider; each maps the neutral `ChatRequest`/`ChatResponse`
//! contract to its vendor wire format (dev/06 §2.1) and nothing vendor-shaped
//! leaks above this module:
//!
//! * [`openai`] — OpenAI Chat Completions + OpenAI-compatible gateways (T059).
//! * [`anthropic`] — Anthropic Messages API (T060).
//! * [`ollama`] — local Ollama daemon, OpenAI-compatible route (T062).
//! * [`local_onnx`] — in-process bundled generative model (T063).

pub mod anthropic;
pub mod local_onnx;
pub mod ollama;
pub mod openai;
