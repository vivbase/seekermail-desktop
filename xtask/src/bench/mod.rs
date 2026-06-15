//! Bench harness orchestration (T055). Runs M1–M8 per `dev/04` and assembles
//! `bench-report.json` (§3c format). Each metric module measures honestly
//! against the seeded corpus where the protocol allows it in-process; metrics
//! that need the packaged app binary (M1) degrade to `"skipped"` unless
//! `--app-binary` is provided — a skipped P0 metric FAILS the gate on the
//! release runner (skip is only green in `--smoke` CI runs).

pub mod gate;
pub mod m1_coldstart;
pub mod m2_list_paint;
pub mod m3_scroll_fps;
pub mod m4_keyword;
pub mod m5_semantic;
pub mod m6_memory;
pub mod m7_index_leak;
pub mod m8_attachment;
pub mod seed;

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::{json, Value};

/// Shared context handed to every metric harness.
pub struct BenchContext {
    /// Seeded corpus database (from `bench-seed`).
    pub corpus_db: PathBuf,
    /// CI smoke mode: shortened M6/M7 windows, skipped metrics allowed.
    pub smoke: bool,
    /// Optional packaged app for M1 cold-start.
    pub app_binary: Option<PathBuf>,
}

/// P0 thresholds (dev/04 §2). Single source for harnesses + initial baseline.
pub mod thresholds {
    pub const M1_COLDSTART_MS: f64 = 1500.0;
    pub const M2_LIST_PAINT_MS: f64 = 400.0;
    pub const M3_DROP_RATE_PCT: f64 = 5.0;
    pub const M4_KEYWORD_MS: f64 = 200.0;
    pub const M5_SEMANTIC_MS: f64 = 1000.0;
    pub const M6_PEAK_RSS_MB: f64 = 500.0;
    pub const M7_GROWTH_MB_PER_DAY: f64 = 50.0;
    pub const M8_LONG_TASK_MS: f64 = 50.0;
}

/// Percentile over an unsorted sample (nearest-rank).
pub fn percentile(samples: &mut [f64], pct: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rank = ((pct / 100.0) * samples.len() as f64).ceil() as usize;
    samples[rank.clamp(1, samples.len()) - 1]
}

/// Time one closure in milliseconds.
pub fn time_ms<T>(f: impl FnOnce() -> T) -> (f64, T) {
    let start = Instant::now();
    let out = f();
    (start.elapsed().as_secs_f64() * 1000.0, out)
}

/// pass/fail status string for a "lower is better" metric.
pub fn status_for(value: f64, threshold: f64) -> &'static str {
    if value <= threshold {
        "pass"
    } else {
        "fail"
    }
}

/// Run every harness and assemble the report object.
pub fn run_all(ctx: &BenchContext) -> Result<Value> {
    let corpus_hash = seed::corpus_checksum(&ctx.corpus_db)
        .context("corpus checksum — run `cargo xtask bench-seed` first")?;

    let metrics = json!({
        "M1": m1_coldstart::run(ctx)?,
        "M2": m2_list_paint::run(ctx)?,
        "M3": m3_scroll_fps::run(ctx)?,
        "M4": m4_keyword::run(ctx)?,
        "M5": m5_semantic::run(ctx)?,
        "M6": m6_memory::run(ctx)?,
        "M7": m7_index_leak::run(ctx)?,
        "M8": m8_attachment::run(ctx)?,
    });

    let gate_result = overall_status(&metrics, ctx.smoke);
    Ok(json!({
        "run_at": chrono::Utc::now().to_rfc3339(),
        "corpus_hash": corpus_hash,
        "tier": "A",
        "smoke": ctx.smoke,
        "metrics": metrics,
        "gate_result": gate_result,
    }))
}

/// green when every metric passes; red when any fails (skips fail outside smoke).
fn overall_status(metrics: &Value, smoke: bool) -> &'static str {
    let mut green = true;
    if let Some(map) = metrics.as_object() {
        for (_, m) in map {
            match m["status"].as_str().unwrap_or("fail") {
                "pass" => {}
                "skipped" if smoke => {}
                _ => green = false,
            }
        }
    }
    if green {
        "green"
    } else {
        "red"
    }
}

/// `cargo xtask bench` CLI.
pub fn cli_bench(args: &[String]) -> Result<u8> {
    let mut out = PathBuf::from("bench-report.json");
    let mut baseline: Option<PathBuf> = None;
    let mut smoke = false;
    let mut app_binary: Option<PathBuf> = None;
    let mut corpus_db: Option<PathBuf> = None;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = PathBuf::from(it.next().context("--out needs a path")?),
            "--baseline" => {
                baseline = Some(PathBuf::from(it.next().context("--baseline needs a path")?));
            }
            "--smoke" => smoke = true,
            "--app-binary" => {
                app_binary = Some(PathBuf::from(
                    it.next().context("--app-binary needs a path")?,
                ));
            }
            "--corpus-db" => {
                corpus_db = Some(PathBuf::from(
                    it.next().context("--corpus-db needs a path")?,
                ));
            }
            other => anyhow::bail!("unknown bench flag: {other}"),
        }
    }

    let ctx = BenchContext {
        corpus_db: corpus_db.unwrap_or_else(seed::default_db_path),
        smoke,
        app_binary,
    };
    let report = run_all(&ctx)?;
    std::fs::write(&out, serde_json::to_string_pretty(&report)?)
        .with_context(|| format!("write {}", out.display()))?;
    eprintln!(
        "bench: report written to {} (gate_result={})",
        out.display(),
        report["gate_result"].as_str().unwrap_or("?")
    );

    match baseline {
        Some(b) if b.exists() => gate::compare_files(&b, &out),
        Some(b) => {
            eprintln!("bench: baseline {} not found — skipping gate", b.display());
            Ok(0)
        }
        None => Ok(if report["gate_result"] == "green" {
            0
        } else {
            1
        }),
    }
}

/// Open the corpus DB read-only.
pub fn open_corpus(path: &Path) -> Result<rusqlite::Connection> {
    rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| {
            format!(
                "open corpus db {} — run `cargo xtask bench-seed`",
                path.display()
            )
        })
}
