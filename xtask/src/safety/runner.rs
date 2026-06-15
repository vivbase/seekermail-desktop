//! `cargo xtask safety-run` (T104 §3c/§3d): evaluate every fixture, compute the
//! safety metrics, and emit `safety-report.json`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::gate;
use super::{evaluate, load_fixtures, Fixture, FIXTURES_JSON};

/// Computed metrics + the per-fixture mismatches.
pub struct Outcome {
    pub fixture_count: usize,
    pub misfire_rate_pct: f64,
    pub sensitive_downgrade_rate_pct: f64,
    pub skip_accuracy_pct: f64,
    pub failures: Vec<(String, String, String)>, // (id, expected, actual)
}

/// Evaluate all fixtures against the safety rules and tally the metrics.
pub fn measure(fixtures: &[Fixture]) -> Outcome {
    let total = fixtures.len().max(1) as f64;

    let mut misfires = 0usize;
    let mut demoted_actual = 0usize;
    let mut skip_expected = 0usize;
    let mut skip_correct = 0usize;
    let mut failures = Vec::new();

    for fx in fixtures {
        let actual = evaluate(fx);
        if actual == "demoted" {
            demoted_actual += 1;
        }
        if fx.expected_outcome == "skipped" {
            skip_expected += 1;
            if actual == "skipped" {
                skip_correct += 1;
            }
        }
        // Misfire: should have been held back but would auto-send.
        if (fx.expected_outcome == "demoted" || fx.expected_outcome == "skipped")
            && actual == "sent_auto"
        {
            misfires += 1;
        }
        if actual != fx.expected_outcome {
            failures.push((
                fx.id.clone(),
                fx.expected_outcome.clone(),
                actual.to_string(),
            ));
        }
    }

    Outcome {
        fixture_count: fixtures.len(),
        misfire_rate_pct: round1(misfires as f64 / total * 100.0),
        sensitive_downgrade_rate_pct: round1(demoted_actual as f64 / total * 100.0),
        skip_accuracy_pct: if skip_expected == 0 {
            100.0
        } else {
            round1(skip_correct as f64 / skip_expected as f64 * 100.0)
        },
        failures,
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn fixtures_checksum() -> String {
    let mut h = Sha256::new();
    h.update(FIXTURES_JSON.as_bytes());
    format!("{:x}", h.finalize())
}

/// Build the `safety-report.json` value (stable shape for T107, §3d).
pub fn build_report(out: &Outcome) -> Value {
    let (misfire_pass, downgrade_pass, overall) =
        gate::evaluate_gate(out.misfire_rate_pct, out.sensitive_downgrade_rate_pct);
    let reason_for = |actual: &str| match actual {
        "sent_auto" => "forced_rule_missed",
        "demoted" => "over_demoted",
        _ => "outcome_mismatch",
    };
    json!({
        "run_at": chrono::Utc::now().to_rfc3339(),
        "fixture_count": out.fixture_count,
        "fixture_checksum": fixtures_checksum(),
        "metrics": {
            "misfire_rate_pct": out.misfire_rate_pct,
            "sensitive_downgrade_rate_pct": out.sensitive_downgrade_rate_pct,
            "skip_accuracy_pct": out.skip_accuracy_pct,
        },
        "gate": {
            "misfire_threshold_pct": gate::MISFIRE_MAX,
            "sensitive_downgrade_min_pct": gate::DOWNGRADE_MIN,
            "sensitive_downgrade_max_pct": gate::DOWNGRADE_MAX,
            "misfire_pass": misfire_pass,
            "sensitive_downgrade_in_range_pass": downgrade_pass,
            "overall": overall,
        },
        "failures": out.failures.iter().map(|(id, expected, actual)| json!({
            "fixture_id": id,
            "expected": expected,
            "actual": actual,
            "reason": reason_for(actual),
        })).collect::<Vec<_>>(),
    })
}

/// `cargo xtask safety-run [--out PATH]`.
pub fn cli(args: &[String]) -> Result<u8> {
    let mut out = PathBuf::from("safety-report.json");
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = PathBuf::from(it.next().context("--out needs a path")?),
            other => anyhow::bail!("unknown safety-run flag: {other}"),
        }
    }

    let fixtures = load_fixtures().context("load fixtures")?;
    let outcome = measure(&fixtures);
    let report = build_report(&outcome);
    std::fs::write(&out, serde_json::to_string_pretty(&report)?)
        .with_context(|| format!("write {}", out.display()))?;
    println!(
        "[safety-run] {} fixtures · misfire {:.1}% · downgrade {:.1}% · {} → {}",
        outcome.fixture_count,
        outcome.misfire_rate_pct,
        outcome.sensitive_downgrade_rate_pct,
        out.display(),
        report["gate"]["overall"].as_str().unwrap_or("?"),
    );
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx(
        id: &str,
        expected: &str,
        attach: bool,
        amount: bool,
        important: bool,
        bulk: bool,
    ) -> Fixture {
        Fixture {
            id: id.into(),
            subject: "s".into(),
            body_snippet: "b".into(),
            sender: "a@x.com".into(),
            has_attachment: attach,
            has_amount: amount,
            important_contact: important,
            is_bulk: bulk,
            expected_outcome: expected.into(),
        }
    }

    #[test]
    fn rules_match_expected_labels() {
        let set = vec![
            fx("a", "sent_auto", false, false, false, false),
            fx("b", "demoted", true, false, false, false),
            fx("c", "demoted", false, true, false, false),
            fx("d", "skipped", false, false, false, true),
        ];
        let m = measure(&set);
        assert_eq!(m.misfire_rate_pct, 0.0);
        assert!(m.failures.is_empty());
        assert_eq!(m.sensitive_downgrade_rate_pct, 50.0); // 2 of 4 demoted
        assert_eq!(m.skip_accuracy_pct, 100.0);
    }

    #[test]
    fn a_mislabelled_sensitive_mail_counts_as_misfire() {
        // Expected demoted, but no forced signal → rules say sent_auto → misfire.
        let set = vec![fx("x", "demoted", false, false, false, false)];
        let m = measure(&set);
        assert_eq!(m.misfire_rate_pct, 100.0);
        assert_eq!(m.failures.len(), 1);
        assert_eq!(
            m.failures[0],
            ("x".into(), "demoted".into(), "sent_auto".into())
        );
    }

    #[test]
    fn report_has_stable_shape() {
        let set = vec![fx("a", "sent_auto", false, false, false, false)];
        let report = build_report(&measure(&set));
        assert!(report["metrics"]["misfire_rate_pct"].is_number());
        assert!(report["gate"]["overall"].is_string());
        assert!(report["fixture_checksum"].is_string());
    }
}
