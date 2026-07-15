// This build script is a thin wrapper around tauri_build::build().
// All cross-language build orchestration (e.g. building the UI
// before the Rust crate compiles) lives in the `Makefile` at the
// repo root; this file only handles the per-crate concerns that
// tauri-build manages (config validation, IPC permission file
// generation, etc.). Run `make build-ui` once before the first
// `cargo check` / `clippy` / `test` on a fresh checkout.
fn main() {
    tauri_build::build()
}
