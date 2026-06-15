//! `embed()` latency benchmark (T030 §8).
//!
//! Measures single-string embedding at three text sizes (~50 / ~200 / ~400 words).
//! Per ADR-0003 + dev/04 M5, GPU-accelerated bge-m3 is the *target*; CPU-only is a
//! soft goal, not a hard gate. By default this runs against the deterministic
//! offline backend; `cargo bench --features local-embed` measures the real runtime.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use seekermail_lib::config::Paths;
use seekermail_lib::embedding::Embedder;

fn make_text(words: usize) -> String {
    // A realistic-ish lexical mix so the offline feature-hasher does real work.
    const LEX: [&str; 16] = [
        "invoice",
        "payment",
        "contract",
        "review",
        "quarterly",
        "report",
        "meeting",
        "schedule",
        "attached",
        "regards",
        "deadline",
        "proposal",
        "budget",
        "client",
        "follow",
        "approval",
    ];
    (0..words)
        .map(|i| LEX[i % LEX.len()])
        .collect::<Vec<_>>()
        .join(" ")
}

fn bench_embed(c: &mut Criterion) {
    let paths = Paths::resolve().expect("resolve paths");
    let embedder = Embedder::load(&paths);

    let mut group = c.benchmark_group("embed");
    for &words in &[50usize, 200, 400] {
        let text = make_text(words);
        group.bench_with_input(BenchmarkId::from_parameter(words), &text, |b, t| {
            b.iter(|| {
                let v = embedder.embed(criterion::black_box(t)).expect("embed");
                criterion::black_box(v.len());
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_embed);
criterion_main!(benches);
