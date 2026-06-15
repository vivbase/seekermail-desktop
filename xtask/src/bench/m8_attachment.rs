//! M8 — attachment download must not block (dev/04 §2: main-thread long task
//! ≤ 50 ms).
//!
//! Protocol: stream a 25 MB stub to disk on a background thread (the T025
//! download path shape) while the "main thread" runs a 60 Hz tick loop; the
//! longest observed tick stall stands in for the longest main-thread long
//! task. P95 over runs gates.

use std::io::Write;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Value};

use super::{percentile, status_for, thresholds, BenchContext};

const STUB_BYTES: usize = 25 * 1024 * 1024;
const CHUNK: usize = 256 * 1024;
const RUNS: usize = 5;
const TICK: Duration = Duration::from_millis(16);

pub fn run(ctx: &BenchContext) -> Result<Value> {
    let runs = if ctx.smoke { 2 } else { RUNS };
    let dir = std::env::temp_dir().join("seekermail-bench-m8");
    std::fs::create_dir_all(&dir)?;

    let mut samples: Vec<f64> = Vec::with_capacity(runs);
    for i in 0..runs {
        let path = dir.join(format!("stub-{i}.bin"));
        let writer_path = path.clone();

        // Background "download": chunked writes, fsync at the end (T025 shape).
        let writer = std::thread::spawn(move || -> std::io::Result<()> {
            let mut f = std::fs::File::create(&writer_path)?;
            let chunk = vec![0xA5u8; CHUNK];
            let mut written = 0usize;
            while written < STUB_BYTES {
                f.write_all(&chunk)?;
                written += CHUNK;
            }
            f.sync_all()
        });

        // Foreground 60 Hz tick loop: the longest tick overshoot = long task.
        let mut longest_ms: f64 = 0.0;
        let mut last = Instant::now();
        while !writer.is_finished() {
            std::thread::sleep(TICK);
            let elapsed = last.elapsed();
            let stall = elapsed.saturating_sub(TICK).as_secs_f64() * 1000.0;
            longest_ms = longest_ms.max(stall);
            last = Instant::now();
        }
        writer
            .join()
            .map_err(|_| anyhow::anyhow!("writer thread panicked"))??;
        std::fs::remove_file(&path).ok();
        samples.push(longest_ms);
    }
    std::fs::remove_dir_all(&dir).ok();

    let p95 = percentile(&mut samples, 95.0);
    Ok(json!({
        "long_task_p95_ms": p95,
        "threshold_ms": thresholds::M8_LONG_TASK_MS,
        "stub_bytes": STUB_BYTES,
        "runs": runs,
        "status": status_for(p95, thresholds::M8_LONG_TASK_MS),
    }))
}
