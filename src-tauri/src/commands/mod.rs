//! Command layer — thin `#[tauri::command]` wrappers (one file per module).
//!
//! Convention (03 §1, 02 "How to read"): a command only (1) deserializes params,
//! (2) calls exactly one service method, (3) maps `AppError → IpcError`. No
//! business logic here. New modules add a file and a `pub use` below, and the
//! command is registered in `lib.rs`'s `generate_handler!`.

pub mod accounts;
pub mod agents;
pub mod ai;
pub mod ai_recommended;
pub mod ai_roles;
pub mod data_flow;
pub mod draft;
pub mod export;
pub mod extraction;
pub mod gte;
pub mod im;
pub mod mail;
pub mod queries;
pub mod reindex;
pub mod search;
pub mod settings;
pub mod style;
pub mod sync_range;
pub mod system;
pub mod wipe;

pub use system::ping;
