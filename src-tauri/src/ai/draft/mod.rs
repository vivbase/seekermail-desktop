//! Module E draft generation (T077/T079/T082/T085).
//!
//! * [`prompt_builder`] — the shared E1/E2/E3 prompt assembly (T079): role +
//!   GTE context (T074), style block (T076), thread snippets, and the task
//!   instruction, packed into one provider-agnostic `ChatRequest`.
//! * [`cleaner`] — pure post-processing of provider output (T077 §3): fence
//!   stripping and trailing-signature dedup, shared by every E-mode.
//! * [`engine`] — the generation entry points; v0.5 ships `generate_e1`
//!   (explicit user trigger), E2/E3 pipelines build on it (T082/T085).
//! * [`repo`] — the `ai_drafts` lifecycle authority (T080): single
//!   INSERT/SELECT statements and the guarded state transitions.
//! * [`expiry`] — the 30-minute background sweep marking lapsed pending
//!   drafts `expired` (T080, F_E6 §4.5).

pub mod cleaner;
pub mod engine;
pub mod expiry;
pub mod prompt_builder;
pub mod repo;

pub use cleaner::clean_ai_body;
pub use engine::{generate_e1, regenerate};
pub use prompt_builder::{BuiltPrompt, DraftPromptBuilder, TriggerMode};
