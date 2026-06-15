//! System commands. `ping` is the first end-to-end IPC roundtrip (T002) and the
//! template every later command follows: a thin wrapper over a service fn that
//! returns `AppResult<T>`, mapped to `Result<T, IpcError>` at the boundary (T004).

use crate::error::{AppResult, IpcError};
use crate::types::PingReply;

/// Service-layer worker. Trivial here, but it models the "command stays thin,
/// logic lives behind `AppResult`" rule so the pattern is copyable.
async fn do_ping() -> AppResult<PingReply> {
    Ok(PingReply {
        message: "pong".to_string(),
    })
}

/// Liveness check — the frontend calls this on mount to prove the IPC bridge.
#[tauri::command]
pub async fn ping() -> Result<PingReply, IpcError> {
    do_ping().await.map_err(IpcError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ping_returns_pong() {
        assert_eq!(do_ping().await.unwrap().message, "pong");
    }
}
