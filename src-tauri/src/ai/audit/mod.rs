//! E7 auto-reply audit log (T088, F_E7).
//!
//! * [`logger`] — [`AuditLogger`]: the append-only write API every E-mode
//!   pipeline uses (fire-and-forget `log`, awaitable `log_await`).
//! * [`types`] — the decision-type vocabulary and the wire DTOs.
//! * [`repo`] — the single `ai_decisions` INSERT + the E7 query, summary, and
//!   export surface.
//! * [`retention`] — the daily policy purge (the one sanctioned DELETE path).

pub mod logger;
pub mod repo;
pub mod retention;
pub mod types;

pub use logger::{AuditEntry, AuditLogger};
pub use types::decision_type;
