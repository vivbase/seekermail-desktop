//! Authorization-level enforcement (T087, AI_MODES §2.3/§7.3/§7.4).
//!
//! `accounts.auth_level` is the authoritative column; `account_ai_settings
//! .auth_level` is its mirror and both are updated in the same transaction by
//! `update_account_ai_settings` (commands/ai.rs `do_update`). This module owns
//! the two read-side primitives every E-mode pipeline shares:
//!
//! * [`router::resolve_auth_route`] — the side-effect-free dispatch read.
//!   Background pipelines (E2 T082, E3 T085) call it at their entry and
//!   silently *skip* when the decision doesn't match their mode — a Manual
//!   account flowing through the E2 pipeline is a normal state, not an error.
//! * [`guards::require_auth_level`] — the hard `FORBIDDEN` gate for explicit
//!   frontend requests that must not run below the required level (e.g. a
//!   direct `run_e2` invocation against a Manual-only account).

pub mod guards;
pub mod router;

pub use guards::require_auth_level;
pub use router::{resolve_auth_route, AuthRouteDecision};
