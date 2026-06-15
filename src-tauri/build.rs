// Tauri build script. The specta TypeScript bindings are NOT exported from here
// (a build script cannot call into the crate's own type modules); they are emitted
// by the `gen-bindings` binary via `pnpm gen:types`. See CONTRIBUTING.md.
fn main() {
    tauri_build::build();
}
