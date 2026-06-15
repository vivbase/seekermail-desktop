//! M3 — scroll dropped-frame rate (dev/04 §2: P95 drop rate < 5 %).
//!
//! Per dev/04 §3 and T055 §4 the CI harness simulates the virtual-list render
//! loop criterion-style (no WebView): each simulated frame materialises one
//! viewport (20 rows) at a random scroll offset through a prepared statement —
//! the same per-frame data work the L0 list does. A frame slower than the
//! 16.67 ms budget counts as dropped. Real-WebView FPS capture stays a manual
//! release-runner verification.

use anyhow::Result;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{json, Value};

use super::{open_corpus, percentile, status_for, thresholds, time_ms, BenchContext};

const FRAME_BUDGET_MS: f64 = 1000.0 / 60.0;
const VIEWPORT_ROWS: i64 = 20;
const FRAMES_PER_RUN: usize = 600; // ~10 s of simulated scrolling
const RUNS: usize = 5;

pub fn run(ctx: &BenchContext) -> Result<Value> {
    let conn = open_corpus(&ctx.corpus_db)?;
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM mails", [], |r| r.get(0))?;
    let max_offset = (total - VIEWPORT_ROWS).max(0);

    let (runs, frames) = if ctx.smoke {
        (2, 120)
    } else {
        (RUNS, FRAMES_PER_RUN)
    };
    let mut rng = StdRng::seed_from_u64(7);
    let mut drop_rates: Vec<f64> = Vec::with_capacity(runs);

    let mut stmt = conn.prepare(
        "SELECT id, subject, from_email, date_sent FROM mails \
         ORDER BY date_sent DESC LIMIT ?1 OFFSET ?2",
    )?;

    for _ in 0..runs {
        let mut dropped = 0usize;
        for _ in 0..frames {
            let offset = rng.gen_range(0..=max_offset);
            let (ms, res) = time_ms(|| -> Result<usize> {
                let rows = stmt
                    .query_map([VIEWPORT_ROWS, offset], |r| {
                        Ok(format!(
                            "{}|{}|{}",
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, i64>(3)?
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows.len())
            });
            res?;
            if ms > FRAME_BUDGET_MS {
                dropped += 1;
            }
        }
        drop_rates.push(dropped as f64 / frames as f64 * 100.0);
    }

    let p95 = percentile(&mut drop_rates, 95.0);
    Ok(json!({
        "drop_rate_p95_pct": p95,
        "threshold_pct": thresholds::M3_DROP_RATE_PCT,
        "frames_per_run": frames,
        "runs": runs,
        "status": status_for(p95, thresholds::M3_DROP_RATE_PCT),
    }))
}
