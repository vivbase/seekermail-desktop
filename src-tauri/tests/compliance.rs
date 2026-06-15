//! BYO-AI compliance test suite (T103).
//!
//! Two automated gates required before any AI release (ADR-0004, dev/09 §5):
//!   * `noproxy_egress` — AI inference egress hosts are the user-configured
//!     provider, never a SeekerMail-controlled domain.
//!   * `log_safety` — secret-bearing values (API keys, prompts, bodies) never
//!     reach the `tracing` output or a `Debug` rendering.
//!
//! Cargo compiles `tests/compliance.rs` as one integration-test crate. Because a
//! test crate root resolves submodules relative to `tests/`, the `#[path]`
//! attributes point at the cases under `tests/compliance/` (the card's layout).

#[path = "compliance/log_safety.rs"]
mod log_safety;
#[path = "compliance/noproxy_egress.rs"]
mod noproxy_egress;
