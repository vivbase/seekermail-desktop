//! M4 — keyword search latency (dev/04 §2: P95 < 200 ms).
//!
//! Protocol (dev/04 §3): 200 pre-generated queries from the corpus word table
//! run against the FTS5 index; the SQL span and the end-to-end (query +
//! hydration) time are recorded separately as sub-breakdowns; P95 gates.

use anyhow::Result;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{json, Value};

use super::{open_corpus, percentile, status_for, thresholds, time_ms, BenchContext};

const QUERIES: usize = 200;
const RESULT_LIMIT: i64 = 50;
/// Same word table the seeder uses — queries always have hits.
const QUERY_WORDS: &[&str] = &[
    "invoice",
    "contract",
    "quarterly",
    "review",
    "meeting",
    "budget",
    "approval",
    "deadline",
    "proposal",
    "renewal",
    "payment",
    "vendor",
    "client",
    "project",
    "policy",
    "estimate",
];

pub fn run(ctx: &BenchContext) -> Result<Value> {
    let conn = open_corpus(&ctx.corpus_db)?;
    let n_queries = if ctx.smoke { 20 } else { QUERIES };
    let mut rng = StdRng::seed_from_u64(11);

    let mut sql_samples: Vec<f64> = Vec::with_capacity(n_queries);
    let mut e2e_samples: Vec<f64> = Vec::with_capacity(n_queries);

    for _ in 0..n_queries {
        let term = QUERY_WORDS[rng.gen_range(0..QUERY_WORDS.len())];
        let two_terms = rng.gen_bool(0.4);
        let query = if two_terms {
            format!(
                "{term} {}",
                QUERY_WORDS[rng.gen_range(0..QUERY_WORDS.len())]
            )
        } else {
            term.to_string()
        };

        let (e2e_ms, sql_ms) = {
            let (total, inner) = time_ms(|| -> Result<f64> {
                // SQL span: the FTS MATCH itself.
                let (sql_ms, ids) = time_ms(|| -> Result<Vec<i64>> {
                    let mut stmt = conn.prepare_cached(
                        "SELECT rowid FROM mails_fts WHERE mails_fts MATCH ?1 \
                         ORDER BY bm25(mails_fts) LIMIT ?2",
                    )?;
                    let ids = stmt
                        .query_map(rusqlite::params![query, RESULT_LIMIT], |r| r.get(0))?
                        .collect::<rusqlite::Result<Vec<i64>>>()?;
                    Ok(ids)
                });
                // Hydration span: pull display rows for the hits.
                let ids = ids?;
                if !ids.is_empty() {
                    let mut stmt = conn.prepare_cached(
                        "SELECT id, subject, from_email, date_sent FROM mails WHERE rowid = ?1",
                    )?;
                    for id in &ids {
                        let _row = stmt.query_row([id], |r| r.get::<_, String>(0))?;
                    }
                }
                Ok(sql_ms)
            });
            (total, inner?)
        };
        sql_samples.push(sql_ms);
        e2e_samples.push(e2e_ms);
    }

    let p50 = percentile(&mut e2e_samples.clone(), 50.0);
    let p95 = percentile(&mut e2e_samples, 95.0);
    let sql_p95 = percentile(&mut sql_samples, 95.0);
    Ok(json!({
        "p50_ms": p50,
        "p95_ms": p95,
        "threshold_ms": thresholds::M4_KEYWORD_MS,
        "queries": n_queries,
        "breakdown": { "sql_p95_ms": sql_p95 },
        "status": status_for(p95, thresholds::M4_KEYWORD_MS),
    }))
}
