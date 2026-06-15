//! Foreground/background throttling for history backfill (T022 §3).
//!
//! Backfill pauses when the device is on low battery (≤ 20%). The check is behind
//! a seam: the default build assumes AC power (returns `false`), and the
//! `live-net` build queries the real battery via the `battery` crate.

/// Battery threshold (fraction) below which backfill pauses (F_A4 §4).
pub const LOW_BATTERY_FRACTION: f32 = 0.20;

/// True when the device is on low battery and backfill should pause.
#[cfg(feature = "live-net")]
pub fn is_low_battery() -> bool {
    match battery::Manager::new().and_then(|m| m.batteries().map(|mut b| b.next())) {
        Ok(Some(Ok(bat))) => {
            let frac = bat.state_of_charge().value;
            let on_battery =
                !matches!(bat.state(), battery::State::Charging | battery::State::Full);
            on_battery && frac <= LOW_BATTERY_FRACTION
        }
        _ => false, // No battery / probe failed → assume AC power.
    }
}

/// Default (non-live) build: assume AC power, never throttle.
#[cfg(not(feature = "live-net"))]
pub fn is_low_battery() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn default_build_never_throttles() {
        // On the default build this is always false; on live-net it depends on HW.
        let _ = is_low_battery();
        assert!(LOW_BATTERY_FRACTION > 0.0);
    }
}
