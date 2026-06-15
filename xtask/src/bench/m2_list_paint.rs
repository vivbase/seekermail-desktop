//! M2 — first list screen over 100k mails (dev/04 §2: P95 < 400 ms).
//!
//! Protocol: the exact L0 first-screen read — newest 100 rows by `date_sent`
//! over the full corpus — including row materialisation, 20 runs, P95.
//! (IPC + render overhead is covered by the M2 margin per dev/04 §3.)

use anyhow::Result;
use serde_json::{json, Value};

use super::{open_corpus, percentile, status_for, thresholds, time_ms, BenchContext};

const RUNS: usize = 20;
const FIRST_SCREEN_ROWS: usize = 100;

pub fn run(ctx: &BenchContext) -> Result<Value> {
    let conn = open_corpus(&ctx.corpus_db)?;
    let runs = if ctx.smoke { 5 } else { RUNS };
    let mut samples = Vec::with_capacity(runs);

    for _ in 0..runs {
        let (ms, rows) = time_ms(|| -> Result<usize> {
            let mut stmt = conn.prepare(
                "SELECT id, account_id, subject, from_email, date_sent, has_attachments \
                 FROM mails ORDER BY date_sent DESC LIMIT ?1",
            )?;
            let rows = stmt
                .query_map([FIRST_SCREEN_ROWS as i64], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(4)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.len())
        });
        anyhow::ensure!(
            rows? == FIRST_SCREEN_ROWS.min(count_mails(&conn)?),
            "short read"
        );
        samples.push(ms);
    }

    let p50 = percentile(&mut samples.clone(), 50.0);
    let p95 = percentile(&mut samples, 95.0);
    Ok(json!({
        "p50_ms": p50,
        "p95_ms": p95,
        "threshold_ms": thresholds::M2_LIST_PAINT_MS,
        "runs": runs,
        "status": status_for(p95, thresholds::M2_LIST_PAINT_MS),
    }))
}

fn count_mails(conn: &rusqlite::Connection) -> Result<usize> {
    Ok(conn.query_row("SELECT COUNT(*) FROM mails", [], |r| r.get::<_, i64>(0))? as usize)
}
