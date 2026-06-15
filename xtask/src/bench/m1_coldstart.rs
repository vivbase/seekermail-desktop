//! M1 — cold start to interactive (dev/04 §2: P95 < 1.5 s).
//!
//! Protocol: spawn the packaged binary (`--app-binary`), timing spawn → first
//! stdout/stderr byte (the app logs its `startup` line immediately before the
//! webview becomes interactive) over 20 runs (dev/04 §0 rule 4). Without a
//! binary the metric reports `skipped` — allowed only in `--smoke` CI runs;
//! the release runner must always pass a binary.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::Result;
use serde_json::{json, Value};

use super::{percentile, status_for, thresholds, BenchContext};

const RUNS: usize = 20;
/// Per-run hard cap so a hung binary can't wedge the harness.
const RUN_TIMEOUT_SECS: u64 = 30;

pub fn run(ctx: &BenchContext) -> Result<Value> {
    let Some(binary) = &ctx.app_binary else {
        return Ok(json!({
            "status": "skipped",
            "note": "no --app-binary provided; M1 requires the packaged app",
            "threshold_ms": thresholds::M1_COLDSTART_MS,
        }));
    };

    let runs = if ctx.smoke { 3 } else { RUNS };
    let mut samples: Vec<f64> = Vec::with_capacity(runs);
    for _ in 0..runs {
        let start = Instant::now();
        let mut child = Command::new(binary)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // First byte on either stream = "alive and logging".
        let mut buf = [0u8; 1];
        let mut stdout = child.stdout.take();
        let elapsed = loop {
            if let Some(out) = stdout.as_mut() {
                if matches!(out.read(&mut buf), Ok(n) if n > 0) {
                    break start.elapsed().as_secs_f64() * 1000.0;
                }
            }
            if start.elapsed().as_secs() > RUN_TIMEOUT_SECS {
                break RUN_TIMEOUT_SECS as f64 * 1000.0;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let _ = child.kill();
        let _ = child.wait();
        samples.push(elapsed);
    }

    let p50 = percentile(&mut samples.clone(), 50.0);
    let p95 = percentile(&mut samples.clone(), 95.0);
    let p99 = percentile(&mut samples, 99.0);
    Ok(json!({
        "p50_ms": p50,
        "p95_ms": p95,
        "p99_ms": p99,
        "threshold_ms": thresholds::M1_COLDSTART_MS,
        "runs": runs,
        "status": status_for(p95, thresholds::M1_COLDSTART_MS),
    }))
}
