//! Bindings generator (T003). Run via `pnpm gen:types`, which expands to:
//!
//! ```text
//! cargo run --manifest-path src-tauri/Cargo.toml --features specta-export --example gen_bindings
//! ```
//!
//! Writes `packages/shared/src/bindings.ts` (path relative to `src-tauri/`).

fn main() -> anyhow::Result<()> {
    let out = "../packages/shared/src/bindings.ts";
    seekermail_lib::export::export_bindings(out)?;
    println!("wrote {out}");
    Ok(())
}
