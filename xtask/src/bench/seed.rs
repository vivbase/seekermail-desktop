//! Deterministic 100k-mail corpus generator — `cargo xtask bench-seed`
//! (T055 §3a, dev/04 §4).
//!
//! Fixed RNG seed (42); 3 accounts split 40/40/20 (legal/work/personal);
//! 18 % of mails carry attachment metadata; 6 % have spam_score > 0.8
//! (vectorize-skipped). The generated row stream is SHA-256 hashed and the
//! digest stored next to the DB; a re-run asserts the checksum so corpus
//! drift fails loudly instead of skewing baselines.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rusqlite::Connection;
use sha2::{Digest, Sha256};

/// Fixed seed (dev/04 §4).
pub const CORPUS_SEED: u64 = 42;
/// Default corpus size.
pub const DEFAULT_COUNT: usize = 100_000;
/// Attachment-metadata rate.
const ATTACHMENT_RATE: f64 = 0.18;
/// Spam (score > 0.8) rate — skipped by vectorize.
const SPAM_RATE: f64 = 0.06;
/// Account split: legal/work/personal.
const ACCOUNTS: [(&str, &str, f64); 3] = [
    ("bench-legal", "legal@bench.seekermail.test", 0.40),
    ("bench-work", "work@bench.seekermail.test", 0.40),
    ("bench-personal", "personal@bench.seekermail.test", 0.20),
];

/// Fixed bilingual-ish word table (dev/04 §4: "Lorem-like English/Chinese-mixed
/// list" — kept ASCII-only here so the repo carries no non-English source text;
/// transliterated tokens stand in for the CJK distribution).
const WORDS: &[&str] = &[
    "invoice",
    "contract",
    "quarterly",
    "review",
    "meeting",
    "schedule",
    "budget",
    "approval",
    "shipment",
    "deadline",
    "proposal",
    "renewal",
    "liability",
    "clause",
    "attachment",
    "summary",
    "report",
    "follow",
    "update",
    "confirm",
    "payment",
    "vendor",
    "client",
    "project",
    "draft",
    "signature",
    "policy",
    "notice",
    "agenda",
    "minutes",
    "estimate",
    "delivery",
    "receipt",
    "heyue",
    "fapiao",
    "baojia",
    "huiyi",
    "jihua",
    "yusuan",
    "qianshu",
    "tixing",
];

pub fn default_dir() -> PathBuf {
    PathBuf::from("target/bench/corpus")
}

pub fn default_db_path() -> PathBuf {
    default_dir().join("seekermail_bench.db")
}

fn checksum_path(db: &Path) -> PathBuf {
    db.with_extension("db.sha256")
}

/// Read the stored corpus digest (written by the last successful seed run).
pub fn corpus_checksum(db: &Path) -> Result<String> {
    std::fs::read_to_string(checksum_path(db))
        .map(|s| s.trim().to_string())
        .with_context(|| format!("read corpus checksum next to {}", db.display()))
}

struct GeneratedMail {
    id: String,
    account_id: &'static str,
    from_email: String,
    subject: String,
    body_text: String,
    date_sent: i64,
    has_attachment: bool,
    attachment_bytes: i64,
    spam_score: Option<f64>,
}

fn words(rng: &mut StdRng, n: usize) -> String {
    (0..n)
        .map(|_| WORDS[rng.gen_range(0..WORDS.len())])
        .collect::<Vec<_>>()
        .join(" ")
}

fn generate(count: usize) -> Vec<GeneratedMail> {
    let mut rng = StdRng::seed_from_u64(CORPUS_SEED);
    let mut out = Vec::with_capacity(count);
    // Cumulative account boundaries over the index space keep the split exact.
    let bounds = [
        (count as f64 * ACCOUNTS[0].2) as usize,
        (count as f64 * (ACCOUNTS[0].2 + ACCOUNTS[1].2)) as usize,
    ];
    for i in 0..count {
        let account = if i < bounds[0] {
            ACCOUNTS[0].0
        } else if i < bounds[1] {
            ACCOUNTS[1].0
        } else {
            ACCOUNTS[2].0
        };
        let has_attachment = rng.gen_bool(ATTACHMENT_RATE);
        let spam = rng.gen_bool(SPAM_RATE);
        out.push(GeneratedMail {
            id: format!("bench-mail-{i:06}"),
            account_id: account,
            from_email: format!("sender{}@corpus.test", rng.gen_range(0..500)),
            // Compute the word counts before borrowing `rng` mutably for `words`
            // (a single expression would double-borrow `rng` → E0499 on current
            // rustc).
            subject: {
                let n = rng.gen_range(3..9);
                words(&mut rng, n)
            },
            body_text: {
                let n = rng.gen_range(40..220);
                words(&mut rng, n)
            },
            // Two years of history ending 2026-01-01 (fixed epoch, deterministic).
            date_sent: 1_767_225_600 - rng.gen_range(0..(2 * 365 * 86_400)) as i64,
            has_attachment,
            attachment_bytes: if has_attachment {
                rng.gen_range(10_000..5_000_000)
            } else {
                0
            },
            spam_score: if spam {
                Some(0.81 + rng.gen::<f64>() * 0.19)
            } else {
                None
            },
        });
    }
    out
}

fn digest(mails: &[GeneratedMail]) -> String {
    let mut hasher = Sha256::new();
    for m in mails {
        hasher.update(m.id.as_bytes());
        hasher.update(m.account_id.as_bytes());
        hasher.update(m.subject.as_bytes());
        hasher.update(m.body_text.as_bytes());
        hasher.update(m.date_sent.to_le_bytes());
        hasher.update([u8::from(m.has_attachment)]);
        hasher.update(m.attachment_bytes.to_le_bytes());
        hasher.update(m.spam_score.unwrap_or(0.0).to_le_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn write_db(db_path: &Path, mails: &[GeneratedMail]) -> Result<()> {
    if db_path.exists() {
        std::fs::remove_file(db_path)?;
    }
    let mut conn = Connection::open(db_path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE accounts (id TEXT PRIMARY KEY, email TEXT NOT NULL);
         CREATE TABLE mails (
            id TEXT PRIMARY KEY,
            account_id TEXT NOT NULL,
            from_email TEXT NOT NULL,
            subject TEXT NOT NULL,
            body_text TEXT NOT NULL,
            date_sent INTEGER NOT NULL,
            has_attachments INTEGER NOT NULL,
            spam_score REAL
         );
         CREATE TABLE attachments (
            mail_id TEXT NOT NULL,
            size_bytes INTEGER NOT NULL
         );
         CREATE INDEX idx_mails_account_date ON mails(account_id, date_sent DESC);
         CREATE VIRTUAL TABLE mails_fts USING fts5(subject, body_text, content='mails', content_rowid='rowid');",
    )?;

    let tx = conn.transaction()?;
    for (id, email, _) in ACCOUNTS {
        tx.execute(
            "INSERT INTO accounts (id, email) VALUES (?1, ?2)",
            (id, email),
        )?;
    }
    {
        let mut ins_mail = tx.prepare(
            "INSERT INTO mails (id, account_id, from_email, subject, body_text, date_sent, has_attachments, spam_score) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        let mut ins_att =
            tx.prepare("INSERT INTO attachments (mail_id, size_bytes) VALUES (?1, ?2)")?;
        for m in mails {
            ins_mail.execute((
                &m.id,
                m.account_id,
                &m.from_email,
                &m.subject,
                &m.body_text,
                m.date_sent,
                i64::from(m.has_attachment),
                m.spam_score,
            ))?;
            if m.has_attachment {
                ins_att.execute((&m.id, m.attachment_bytes))?;
            }
        }
    }
    tx.execute_batch("INSERT INTO mails_fts(mails_fts) VALUES('rebuild');")?;
    tx.commit()?;
    Ok(())
}

/// Generate (or re-verify) the corpus. Returns the digest.
pub fn run(count: usize, out_dir: &Path, _with_blobs: bool) -> Result<String> {
    std::fs::create_dir_all(out_dir)?;
    let db_path = out_dir.join("seekermail_bench.db");

    let mails = generate(count);
    let hash = digest(&mails);

    // Determinism assertion: same seed+count must reproduce the stored digest.
    if let Ok(prev) = corpus_checksum(&db_path) {
        // Stored digests embed the count so different sizes don't collide.
        if let Some(prev_hash) = prev.strip_prefix(&format!("{count}:")) {
            anyhow::ensure!(
                prev_hash == hash,
                "corpus checksum mismatch — generator drifted (was {prev_hash}, now {hash})"
            );
        }
    }

    write_db(&db_path, &mails)?;
    std::fs::write(checksum_path(&db_path), format!("{count}:{hash}\n"))?;
    eprintln!(
        "bench-seed: {count} mails → {} (sha256 {hash})",
        db_path.display()
    );
    Ok(hash)
}

/// `cargo xtask bench-seed` CLI.
pub fn cli(args: &[String]) -> Result<u8> {
    let mut count = DEFAULT_COUNT;
    let mut with_blobs = false;
    let mut out_dir = default_dir();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--count" => count = it.next().context("--count needs a number")?.parse()?,
            "--with-blobs" => with_blobs = true,
            "--out" => out_dir = PathBuf::from(it.next().context("--out needs a dir")?),
            other => anyhow::bail!("unknown bench-seed flag: {other}"),
        }
    }
    run(count, &out_dir, with_blobs)?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_digest() {
        let a = digest(&generate(500));
        let b = digest(&generate(500));
        assert_eq!(a, b, "generator must be deterministic");
    }

    #[test]
    fn split_rates_roughly_hold() {
        let mails = generate(10_000);
        let legal = mails
            .iter()
            .filter(|m| m.account_id == "bench-legal")
            .count();
        let spam = mails.iter().filter(|m| m.spam_score.is_some()).count();
        let atts = mails.iter().filter(|m| m.has_attachment).count();
        assert!(
            (3_900..=4_100).contains(&legal),
            "40% legal split, got {legal}"
        );
        assert!((450..=750).contains(&spam), "~6% spam, got {spam}");
        assert!(
            (1_600..=2_000).contains(&atts),
            "~18% attachments, got {atts}"
        );
    }

    #[test]
    fn seed_run_writes_db_and_checksum() {
        let dir = std::env::temp_dir().join(format!("sm-bench-{}", std::process::id()));
        let hash1 = run(200, &dir, false).unwrap();
        let hash2 = run(200, &dir, false).unwrap(); // re-run must match
        assert_eq!(hash1, hash2);
        let db = dir.join("seekermail_bench.db");
        let conn = Connection::open(&db).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM mails", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 200);
        std::fs::remove_dir_all(&dir).ok();
    }
}
