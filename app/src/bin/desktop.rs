//! §X — Desktop binary entry point (Windows, macOS, Linux).
//!
//! ## Why this file exists separately from `lib.rs`
//!
//! `app_lib::run()` is the shared application entry; it is
//! reused by the desktop binary (this file) and, eventually,
//! by Tauri 2's mobile tooling on iOS / Android (via the
//! `#[cfg_attr(mobile, tauri::mobile_entry_point)]` attribute
//! on `run()` — see `lib.rs`). Keeping the entry in `lib.rs`
//! is what lets the same setup code drive every target.
//!
//! What does NOT carry over between targets is the binary's
//! PE subsystem flag. On Windows specifically, the linker
//! reads the `windows_subsystem` attribute from the
//! **binary crate's** `main.rs` to decide whether the
//! produced `.exe` is a CONSOLE app (which allocates a
//! console window on launch — what the v0.1.2 release
//! accidentally did) or a WINDOWS app (no console — what
//! double-clickable GUI apps should be).
//!
//! The previous setup had `windows_subsystem = "windows"` on
//! `app/src/lib.rs`, but Rust only honors the attribute when
//! it appears in the binary's own crate — putting it on the
//! library crate has no effect on the produced `.exe`. That
//! is why the v0.1.2 release opened a console window
//! alongside the Tauri window and kept it open for the
//! lifetime of the process.
//!
//! ## Why the conditional
//!
//! `cfg_attr(all(not(debug_assertions), target_os = "windows"))`
//! applies the attribute **only** in release builds on
//! Windows. In debug builds (`cargo run`) the console stays
//! attached so stdout / stderr are visible live, which is
//! what you want during development. In release builds the
//! console is suppressed so end users double-clicking the
//! installer don't see a black window pop up. macOS and
//! Linux ignore the attribute either way; they have no
//! equivalent subsystem distinction.
//!
//! ## Future mobile targets
//!
//! When we add iOS / Android, Tauri 2's `cargo tauri ios init`
//! and `cargo tauri android init` generate the platform
//! entry points from the `mobile_entry_point` attribute on
//! `app_lib::run()`. We do NOT need a `bin/ios.rs` or
//! `bin/android.rs` here — those would just be dead weight
//! that the mobile build pipeline replaces anyway. If we
//! later need a Rust entry stub for a target that the
//! `mobile_entry_point` macro does not cover, we add it
//! then.
//!
//! ## Why no console for release, and how to debug it
//!
//! Suppressing the console means the v0.1.1 startup panic
//! (and any future panics) would have been invisible to the
//! user — Windows shows no console, so `tracing` output
//! went nowhere. The follow-up §X.Y is to install a
//! `tracing-appender` that writes to a rolling log file
//! under the user's config dir (e.g. `%APPDATA%\talon\logs`
//! on Windows, `~/.config/talon/logs` on Linux, etc.) so
//! crash output is recoverable. That is out of scope for
//! v0.1.3; the user explicitly asked to defer it.

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

fn main() {
    app_lib::run();
}
