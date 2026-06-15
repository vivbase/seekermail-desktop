//! `bench-gate` — baseline comparison (T055 §3a, dev/04 §6).
//!
//! Reads two `bench-report.json` files and compares metric by metric:
//!   * report value over its hard threshold → RED, exit(1)
//!   * report value > baseline × 1.10      → AMBER warning, exit(0)
//!   * otherwise                            → green

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

/// Amber drift factor over baseline (dev/04 §6).
pub const AMBER_FACTOR: f64 = 1.10;

/// The headline field compared per metric.
fn headline(metric: &Value) -> Option<(&'static str, f64, f64)> {
    // (field, value, threshold) — first matching pair wins.
    const CANDIDATES: [(&str, &str); 5] = [
        ("p95_ms", "threshold_ms"),
        ("drop_rate_p95_pct", "threshold_pct"),
        ("peak_rss_mb", "threshold_mb"),
        ("growth_mb_per_day", "threshold_mb_per_day"),
        ("long_task_p95_ms", "threshold_ms"),
    ];
    for (field, thr) in CANDIDATES {
        if let (Some(v), Some(t)) = (metric[field].as_f64(), metric[thr].as_f64()) {
            // Leak the static field name for the message.
            let name = match field {
                "p95_ms" => "p95_ms",
                "drop_rate_p95_pct" => "drop_rate_p95_pct",
                "peak_rss_mb" => "peak_rss_mb",
                "growth_mb_per_day" => "growth_mb_per_day",
                _ => "long_task_p95_ms",
            };
            return Some((name, v, t));
        }
    }
    None
}

/// Compare a report against a baseline. Returns the process exit code.
pub fn compare(baseline: &Value, report: &Value) -> Result<u8> {
    let (Some(base_metrics), Some(rep_metrics)) = (
        baseline["metrics"].as_object(),
        report["metrics"].as_object(),
    ) else {
        anyhow::bail!("both files must contain a top-level \"metrics\" object");
    };

    let mut failed = false;
    let mut amber = false;

    for (name, rep) in rep_metrics {
        let status = rep["status"].as_str().unwrap_or("fail");
        if status == "skipped" {
            eprintln!("[SKIP]  {name} not measured in this run");
            continue;
        }
        let Some((field, value, threshold)) = headline(rep) else {
            eprintln!("[WARN]  {name} has no comparable headline field");
            continue;
        };
        if value > threshold {
            eprintln!("[RED]   {name} {field} = {value:.1} exceeds threshold {threshold:.1}");
            failed = true;
            continue;
        }
        if let Some(base) = base_metrics
            .get(name)
            .and_then(|b| headline(b).map(|(_, v, _)| v))
        {
            if base > 0.0 && value > base * AMBER_FACTOR {
                let pct = (value / base - 1.0) * 100.0;
                eprintln!(
                    "[AMBER] {name} regressed vs baseline ({value:.1} vs {base:.1} baseline, +{pct:.0}%)"
                );
                amber = true;
            }
        }
    }

    if failed {
        eprintln!("bench-gate: RED — at least one P0 metric over threshold");
        Ok(1)
    } else if amber {
        eprintln!("bench-gate: AMBER — within thresholds but regressed >10% vs baseline");
        Ok(0)
    } else {
        eprintln!("bench-gate: green");
        Ok(0)
    }
}

pub fn compare_files(baseline: &Path, report: &Path) -> Result<u8> {
    let b: Value = serde_json::from_str(
        &std::fs::read_to_string(baseline)
            .with_context(|| format!("read baseline {}", baseline.display()))?,
    )?;
    let r: Value = serde_json::from_str(
        &std::fs::read_to_string(report)
            .with_context(|| format!("read report {}", report.display()))?,
    )?;
    compare(&b, &r)
}

/// `cargo xtask bench-gate` CLI.
pub fn cli(args: &[String]) -> Result<u8> {
    let mut baseline: Option<PathBuf> = None;
    let mut report: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--baseline" => {
                baseline = Some(PathBuf::from(it.next().context("--baseline needs a path")?));
            }
            "--report" => {
                report = Some(PathBuf::from(it.next().context("--report needs a path")?));
            }
            other => anyhow::bail!("unknown bench-gate flag: {other}"),
        }
    }
    compare_files(
        &baseline.context("--baseline is required")?,
        &report.context("--report is required")?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report(p95: f64, threshold: f64) -> Value {
        let status = if p95 <= threshold { "pass" } else { "fail" };
        json!({ "metrics": { "M4": {
            "p95_ms": p95, "threshold_ms": threshold, "status": status
        }}})
    }

    #[test]
    fn all_pass_is_exit_zero() {
        let code = compare(&report(150.0, 200.0), &report(150.0, 200.0)).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn over_threshold_is_exit_one() {
        let code = compare(&report(150.0, 200.0), &report(230.0, 200.0)).unwrap();
        assert_eq!(code, 1);
    }

    #[test]
    fn regression_over_ten_percent_is_amber_exit_zero() {
        // 150 → 180: under the 200 threshold, but +20% vs baseline → amber.
        let code = compare(&report(150.0, 200.0), &report(180.0, 200.0)).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn skipped_metric_does_not_fail_gate() {
        let rep = json!({ "metrics": { "M1": { "status": "skipped" } } });
        let code = compare(&json!({"metrics": {}}), &rep).unwrap();
        assert_eq!(code, 0);
    }
}
