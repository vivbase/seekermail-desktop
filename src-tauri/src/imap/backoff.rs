//! Exponential backoff for transient sync failures (T021 §3).
//!
//! Schedule: 1 s, 5 s, then 30 s (capped). Indexed by the count of consecutive
//! errors so the first retry is fast and repeated failures slow down.

use std::time::Duration;

const SCHEDULE_SECS: [u64; 3] = [1, 5, 30];

/// Backoff duration for the Nth consecutive error (0-based).
pub fn next_backoff(consecutive_errors: u32) -> Duration {
    let idx = (consecutive_errors as usize).min(SCHEDULE_SECS.len() - 1);
    Duration::from_secs(SCHEDULE_SECS[idx])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_steps_then_caps() {
        assert_eq!(next_backoff(0), Duration::from_secs(1));
        assert_eq!(next_backoff(1), Duration::from_secs(5));
        assert_eq!(next_backoff(2), Duration::from_secs(30));
        assert_eq!(next_backoff(9), Duration::from_secs(30)); // capped
    }
}
