//! `cargo xtask` — repo automation entry point (T055).
//!
//! Subcommands:
//!   * `bench-seed [--count N] [--with-blobs] [--out DIR]` — deterministic
//!     fixture corpus (seed=42, checksum-asserted).
//!   * `bench [--out PATH] [--baseline PATH] [--smoke] [--app-binary PATH]` —
//!     run the M1–M8 harnesses and emit `bench-report.json`; gate when a
//!     baseline is given.
//!   * `bench-gate --baseline PATH --report PATH` — compare two reports;
//!     exit(1) on any threshold fail, amber warning (exit 0) on >×1.10 drift.
//!   * `safety-seed [--out PATH]` — load the labelled AI-safety fixtures into a
//!     deterministic SQLite DB (T104).
//!   * `safety-run [--out PATH]` — evaluate the fixtures and emit
//!     `safety-report.json` (misfire + sensitive-downgrade metrics).
//!   * `safety-gate --report PATH` — exit(1) unless misfire < 5% and
//!     sensitive-downgrade ∈ [10, 30]%.

mod bench;
mod safety;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let rest = &args[1.min(args.len())..];

    let result = match cmd {
        "bench-seed" => bench::seed::cli(rest),
        "bench" => bench::cli_bench(rest),
        "bench-gate" => bench::gate::cli(rest),
        "safety-seed" => safety::seed::cli(rest),
        "safety-run" => safety::runner::cli(rest),
        "safety-gate" => safety::gate::cli(rest),
        _ => {
            eprintln!(
                "usage: cargo xtask <bench-seed|bench|bench-gate|safety-seed|safety-run|safety-gate> [options]\n\
                 \n\
                 bench-seed   [--count N] [--with-blobs] [--out DIR]\n\
                 bench        [--out PATH] [--baseline PATH] [--smoke] [--app-binary PATH]\n\
                 bench-gate   --baseline PATH --report PATH\n\
                 safety-seed  [--out PATH]\n\
                 safety-run   [--out PATH]\n\
                 safety-gate  --report PATH"
            );
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("xtask error: {e:#}");
            ExitCode::from(1)
        }
    }
}
