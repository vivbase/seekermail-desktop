//! M6 — steady-state memory (dev/04 §2: peak RSS < 500 MB).
//!
//! Protocol (dev/04 §3): a 3-account continuous poll loop with RSS sampling;
//! nightly runs the full window, PR/smoke CI runs a shortened window and
//! extrapolates the slope. Window lengths: nightly 8 h ↔ `BENCH_M6_SECS` env;
//! default non-smoke 30 min; smoke 10 s.

use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Value};
use sysinfo::{Pid, System};

use super::{open_corpus, status_for, thresholds, BenchContext};

const SMOKE_SECS: u64 = 10;
const DEFAULT_SECS: u64 = 30 * 60;

fn window_secs(smoke: bool) -> u64 {
    if let Ok(v) = std::env::var("BENCH_M6_SECS") {
        if let Ok(n) = v.parse() {
            return n;
        }
    }
    if smoke {
        SMOKE_SECS
    } else {
        DEFAULT_SECS
    }
}

/// Current process RSS in MB.
pub fn rss_mb(sys: &mut System) -> f64 {
    let pid = Pid::from_u32(std::process::id());
    sys.refresh_processes();
    sys.process(pid)
        .map(|p| p.memory() as f64 / (1024.0 * 1024.0))
        .unwrap_or(0.0)
}

/// Least-squares slope (MB per hour) over (t_secs, rss_mb) samples.
pub fn slope_mb_per_hour(samples: &[(f64, f64)]) -> f64 {
    let n = samples.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let sx: f64 = samples.iter().map(|(x, _)| x).sum();
    let sy: f64 = samples.iter().map(|(_, y)| y).sum();
    let sxx: f64 = samples.iter().map(|(x, _)| x * x).sum();
    let sxy: f64 = samples.iter().map(|(x, y)| x * y).sum();
    let denom = n * sxx - sx * sx;
    if denom.abs() < f64::EPSILON {
        return 0.0;
    }
    ((n * sxy - sx * sy) / denom) * 3600.0
}

pub fn run(ctx: &BenchContext) -> Result<Value> {
    let conn = open_corpus(&ctx.corpus_db)?;
    let secs = window_secs(ctx.smoke);
    let mut sys = System::new();
    let start = Instant::now();
    let mut samples: Vec<(f64, f64)> = Vec::new();
    let mut peak: f64 = 0.0;

    // Synthetic 3-account poll loop: newest-page reads per account, the same
    // shape the A4 scheduler issues at steady state.
    let mut stmt = conn.prepare(
        "SELECT id, subject, date_sent FROM mails WHERE account_id = ?1 \
         ORDER BY date_sent DESC LIMIT 50",
    )?;
    let accounts = ["bench-legal", "bench-work", "bench-personal"];

    let mut next_sample = Duration::ZERO;
    while start.elapsed().as_secs() < secs {
        for acc in accounts {
            let _rows = stmt
                .query_map([acc], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
        }
        if start.elapsed() >= next_sample {
            let rss = rss_mb(&mut sys);
            peak = peak.max(rss);
            samples.push((start.elapsed().as_secs_f64(), rss));
            next_sample = start.elapsed() + Duration::from_secs(1);
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let trend = slope_mb_per_hour(&samples);
    Ok(json!({
        "peak_rss_mb": peak,
        "threshold_mb": thresholds::M6_PEAK_RSS_MB,
        "trend_mb_per_h": trend,
        "window_secs": secs,
        "extrapolated": secs < 8 * 3600,
        "status": status_for(peak, thresholds::M6_PEAK_RSS_MB),
    }))
}
