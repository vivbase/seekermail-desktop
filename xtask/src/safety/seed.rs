//! `cargo xtask safety-seed` (T104 §3b): load the labelled fixtures into a
//! deterministic SQLite DB and print a stable content checksum.

use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

use super::{load_fixtures, FIXTURES_JSON};

const DEFAULT_DB: &str = "target/safety/seekermail_safety.db";

/// SHA-256 of the embedded fixture JSON — identical across runs (determinism).
pub fn checksum() -> String {
    let mut h = Sha256::new();
    h.update(FIXTURES_JSON.as_bytes());
    format!("{:x}", h.finalize())
}

pub fn cli(args: &[String]) -> Result<u8> {
    let mut out = PathBuf::from(DEFAULT_DB);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = PathBuf::from(it.next().context("--out needs a path")?),
            other => anyhow::bail!("unknown safety-seed flag: {other}"),
        }
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(&out);

    let conn = Connection::open(&out).with_context(|| format!("open {}", out.display()))?;
    conn.execute_batch(
        "CREATE TABLE safety_fixtures (
            id                TEXT PRIMARY KEY,
            subject           TEXT NOT NULL,
            body_snippet      TEXT NOT NULL,
            sender            TEXT NOT NULL,
            has_attachment    INTEGER NOT NULL,
            has_amount        INTEGER NOT NULL,
            important_contact INTEGER NOT NULL,
            is_bulk           INTEGER NOT NULL,
            expected_outcome  TEXT NOT NULL
         );",
    )?;

    let fixtures = load_fixtures().context("parse fixtures")?;
    for fx in &fixtures {
        conn.execute(
            "INSERT INTO safety_fixtures VALUES (?,?,?,?,?,?,?,?,?)",
            params![
                fx.id,
                fx.subject,
                fx.body_snippet,
                fx.sender,
                fx.has_attachment as i64,
                fx.has_amount as i64,
                fx.important_contact as i64,
                fx.is_bulk as i64,
                fx.expected_outcome,
            ],
        )?;
    }

    let sum = checksum();
    println!(
        "[safety-seed] {} fixtures → {} (checksum {})",
        fixtures.len(),
        out.display(),
        &sum[..16],
    );
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_parse_and_count_at_least_100() {
        assert!(load_fixtures().unwrap().len() >= 100);
    }

    #[test]
    fn checksum_is_deterministic() {
        assert_eq!(checksum(), checksum());
    }
}
