//! M5 — semantic search latency (dev/04 §2: P95 < 1 s).
//!
//! Protocol (dev/04 §3): 100 natural-language queries → embed → ANN over the
//! vector index → hydrate the top hits. The harness mirrors the app's offline
//! deterministic embedder (feature hashing) and brute-force-cosine vector
//! backend (the shipped T019 implementation), built once from a corpus subset
//! and queried per-iteration end-to-end with sub-span breakdowns.

use anyhow::Result;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{json, Value};

use super::{open_corpus, percentile, status_for, thresholds, time_ms, BenchContext};

const DIM: usize = 256;
const QUERIES: usize = 100;
const TOP_K: usize = 50;
/// Index size: enough to make cosine scans honest without minutes of setup.
const INDEX_ROWS: usize = 20_000;

/// Feature-hash embed — same scheme as the app's offline embedder.
fn embed(text: &str) -> Vec<f32> {
    let mut v = vec![0.0f32; DIM];
    for token in text.split_whitespace() {
        let mut h: u64 = 1469598103934665603;
        for b in token.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(1099511628211);
        }
        v[(h % DIM as u64) as usize] += 1.0;
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

pub fn run(ctx: &BenchContext) -> Result<Value> {
    let conn = open_corpus(&ctx.corpus_db)?;
    let index_rows = if ctx.smoke { 2_000 } else { INDEX_ROWS };
    let n_queries = if ctx.smoke { 10 } else { QUERIES };

    // Build the in-memory vector index from a corpus subset (setup, not timed).
    let mut stmt = conn.prepare(
        "SELECT rowid, subject || ' ' || body_text FROM mails \
         WHERE spam_score IS NULL LIMIT ?1",
    )?;
    let index: Vec<(i64, Vec<f32>)> = stmt
        .query_map([index_rows as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|(id, text)| (id, embed(&text)))
        .collect();

    let query_templates = [
        "last email about the contract renewal",
        "invoice payment from the vendor",
        "quarterly budget review meeting",
        "project deadline and delivery schedule",
        "policy approval and signature",
    ];
    let mut rng = StdRng::seed_from_u64(13);

    let mut e2e: Vec<f64> = Vec::with_capacity(n_queries);
    let mut embed_spans: Vec<f64> = Vec::with_capacity(n_queries);
    let mut ann_spans: Vec<f64> = Vec::with_capacity(n_queries);

    let mut hydrate = conn.prepare("SELECT id, subject FROM mails WHERE rowid = ?1")?;

    for _ in 0..n_queries {
        let q = format!(
            "{} {}",
            query_templates[rng.gen_range(0..query_templates.len())],
            rng.gen_range(0..1000) // cache-buster variation
        );
        let (total_ms, spans) = time_ms(|| -> Result<(f64, f64)> {
            let (embed_ms, qv) = time_ms(|| embed(&q));
            let (ann_ms, hits) = time_ms(|| {
                let mut scored: Vec<(i64, f32)> =
                    index.iter().map(|(id, v)| (*id, cosine(&qv, v))).collect();
                scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                scored.truncate(TOP_K);
                scored
            });
            for (rowid, _score) in &hits {
                let _ = hydrate.query_row([rowid], |r| r.get::<_, String>(0))?;
            }
            Ok((embed_ms, ann_ms))
        });
        let (embed_ms, ann_ms) = spans?;
        e2e.push(total_ms);
        embed_spans.push(embed_ms);
        ann_spans.push(ann_ms);
    }

    let p50 = percentile(&mut e2e.clone(), 50.0);
    let p95 = percentile(&mut e2e, 95.0);
    Ok(json!({
        "p50_ms": p50,
        "p95_ms": p95,
        "threshold_ms": thresholds::M5_SEMANTIC_MS,
        "queries": n_queries,
        "index_rows": index_rows,
        "breakdown": {
            "embed_p95_ms": percentile(&mut embed_spans, 95.0),
            "ann_p95_ms": percentile(&mut ann_spans, 95.0),
        },
        "status": status_for(p95, thresholds::M5_SEMANTIC_MS),
    }))
}
