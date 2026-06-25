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
pub mod identity;
pub mod im;
pub mod mail;
pub mod memory;
pub mod queries;
pub mod reindex;
pub mod risk;
pub mod search;
pub mod settings;
pub mod shell;
pub mod style;
pub mod sync_range;
pub mod system;
pub mod wipe;
// T2 (WB-12) window commands — verify with `cargo build` / `cargo test --lib` on the Mac (dev/22 §A).
pub mod workbench;

pub use system::ping;
