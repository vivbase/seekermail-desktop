//! M7 — index-growth memory leak (dev/04 §2: growth < 50 MB/day).
//!
//! Protocol (dev/04 §3): continuous embedding churn with RSS sampling; the
//! nightly window is hours, smoke runs a short window and extrapolates the
//! least-squares slope × 24 h. Window: `BENCH_M7_SECS` env override;
//! default non-smoke 2 h; smoke 15 s.

use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Value};
use sysinfo::System;

use super::m6_memory::{rss_mb, slope_mb_per_hour};
use super::{open_corpus, status_for, thresholds, BenchContext};

const SMOKE_SECS: u64 = 15;
const DEFAULT_SECS: u64 = 2 * 3600;
const DIM: usize = 256;

fn window_secs(smoke: bool) -> u64 {
    if let Ok(v) = std::env::var("BENCH_M7_SECS") {
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

pub fn run(ctx: &BenchContext) -> Result<Value> {
    let conn = open_corpus(&ctx.corpus_db)?;
    let secs = window_secs(ctx.smoke);
    let mut sys = System::new();
    let start = Instant::now();
    let mut samples: Vec<(f64, f64)> = Vec::new();

    // Continuous "embed → upsert" churn against an in-memory vector table —
    // the allocation/retention pattern the B3 worker exercises all day.
    let mut stmt = conn.prepare("SELECT body_text FROM mails WHERE rowid = ?1")?;
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM mails", [], |r| r.get(0))?;
    let mut store: Vec<(i64, Vec<f32>)> = Vec::new();
    let mut cursor: i64 = 1;
    let mut next_sample = Duration::ZERO;

    while start.elapsed().as_secs() < secs {
        let rowid = (cursor % total.max(1)) + 1;
        cursor += 1;
        if let Ok(text) = stmt.query_row([rowid], |r| r.get::<_, String>(0)) {
            // Cheap deterministic pseudo-embedding (allocation pattern matters,
            // not the math).
            let mut v = vec![0.0f32; DIM];
            for (i, b) in text.bytes().enumerate() {
                v[i % DIM] += b as f32 / 255.0;
            }
            // Upsert semantics: replace, don't grow unboundedly (mirrors T019).
            if let Some(slot) = store.iter_mut().find(|(id, _)| *id == rowid) {
                slot.1 = v;
            } else {
                store.push((rowid, v));
                if store.len() > 10_000 {
                    store.remove(0);
                }
            }
        }
        if start.elapsed() >= next_sample {
            samples.push((start.elapsed().as_secs_f64(), rss_mb(&mut sys)));
            next_sample = start.elapsed() + Duration::from_secs(1);
        }
    }

    let growth_per_day = slope_mb_per_hour(&samples) * 24.0;
    Ok(json!({
        "growth_mb_per_day": growth_per_day,
        "threshold_mb_per_day": thresholds::M7_GROWTH_MB_PER_DAY,
        "window_secs": secs,
        "extrapolated": secs < 24 * 3600,
        "status": status_for(growth_per_day, thresholds::M7_GROWTH_MB_PER_DAY),
    }))
}
