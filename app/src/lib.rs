//! Tauri app entry. The actual UI is in `ui/`. The Rust side exposes
//! commands (typed IPC handlers) to the webview.
//!
//! The `lib` form (rather than putting everything in `main.rs`) is the
//! Tauri 2 convention: it lets the same `run()` function be called from
//! desktop, iOS, and Android entry points without duplicating setup.

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use serde::Serialize;

/// Payload returned by the `greet` command. Round-tripped to the React
/// `App` component on startup as a sanity check that the IPC bridge is
/// alive.
#[derive(Debug, Clone, Serialize)]
pub struct Greeting {
    /// A friendly message, constant in v0.1. Later phases will make this
    /// include project context (e.g. "Hello, project `acme-bounty`").
    pub message: String,
    /// Engine version, from the Tauri `AppHandle`'s package info.
    pub version: String,
}

/// Tauri command: returns a greeting. The first command every Talon
/// release supports; it proves the bridge works even before any real
/// proxy / storage code is wired up.
#[tauri::command]
fn greet(app: tauri::AppHandle) -> Greeting {
    Greeting {
        message: "Hello from Talon".to_string(),
        version: app.package_info().version.to_string(),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize tracing so the `tracing::info!` in the setup hook and
    // any Tauri-internal logs are visible on stderr. The fmt subscriber
    // is the minimum viable setup; v0.5 will add an env-filter so users
    // can set RUST_LOG=talon=debug to get verbose output.
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    tauri::Builder::default()
        .setup(|app| {
            // On startup, log the engine version so users (and our xvfb
            // smoke tests) can verify the IPC bridge is alive even before
            // opening DevTools.
            tracing::info!(
                version = %app.package_info().version,
                "talon engine started"
            );
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("error while running Talon");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Greeting struct round-trips through JSON correctly. Catches
    /// accidental `pub` removal, missing `Serialize` derives, etc.
    #[test]
    fn greeting_serializes_to_expected_json_shape() {
        let g = Greeting {
            message: "Hello from Talon".to_string(),
            version: "0.1.0".to_string(),
        };
        let v: serde_json::Value = serde_json::to_value(&g).expect("serialize");
        assert_eq!(v["message"], "Hello from Talon");
        assert_eq!(v["version"], "0.1.0");
    }
}
