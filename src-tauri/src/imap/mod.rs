//! IMAP fetch layer (Module A4): scheduling, sync, history backfill, MIME parse,
//! thread resolution, and attachment download.
//!
//! Data flow (F_A4 §2): the scheduler + backfill tasks fetch raw MIME and push it
//! onto the ingest channel; the parse worker decodes + sanitises + persists, then
//! fans out attachments. Everything reaches the network through the transport
//! seam in [`crate::net`], so the whole pipeline is unit-testable with fakes.

pub mod attachment;
pub mod backfill;
pub mod backoff;
pub mod dedup;
pub mod idle_task;
pub mod inbound;
pub mod outbound;
pub mod parser;
pub mod poll_task;
pub mod sampler;
pub mod scheduler;
pub mod sync;
pub mod thread;
pub mod throttle;

pub use scheduler::SyncScheduler;
