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

mod agent;
mod commands;
mod proxy_handle;
mod wire;
mod wire_bus;

use std::path::PathBuf;
use std::sync::Arc;

use bk_engine::Engine;
use serde::Serialize;
use tracing::info;

use crate::commands::EngineArc;
use crate::proxy_handle::{ProxyHandle, ProxyHandleArc};
use crate::wire_bus::WireEventBus;

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

/// Resolves the Talon config directory. Falls back to the
/// system temp dir on platforms where the standard config dir
/// is unavailable.
fn default_config_dir() -> PathBuf {
    system_config_dir()
        .or_else(system_data_dir)
        .unwrap_or_else(|| std::env::temp_dir().join("talon"))
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn system_config_dir() -> Option<PathBuf> {
    // `dirs` is a transitive dep of `bk-store`; use it
    // directly rather than adding a new dep.
    Some(dirs::config_dir()?.join("talon"))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn system_config_dir() -> Option<PathBuf> {
    None
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn system_data_dir() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("talon"))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn system_data_dir() -> Option<PathBuf> {
    None
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

    let config_dir = default_config_dir();
    let engine: EngineArc = match Engine::new(config_dir.clone()) {
        Ok(e) => Arc::new(e),
        Err(err) => {
            eprintln!(
                "talon: failed to create engine at {}: {err}",
                config_dir.display()
            );
            std::process::exit(1);
        }
    };
    let proxy: ProxyHandleArc = Arc::new(ProxyHandle::new(&config_dir));

    tauri::Builder::default()
        .manage(agent::AgentState::new())
        .manage(wire::WireEventSeq::new())
        .manage(WireEventBus::new())
        .manage(engine.clone())
        .manage(proxy.clone())
        .setup(move |app| {
            // §4.2: wire the engine + proxy event buses into
            // the WireEvent fan-in. The setup hook runs once
            // at app start.
            info!(
                version = %app.package_info().version,
                config_dir = %config_dir.display(),
                "talon engine started"
            );
            let app_handle = app.handle().clone();
            wire_bus::setup_wire_bus(&app_handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            commands::core::open_project,
            commands::core::close_project,
            commands::core::list_exchanges,
            commands::core::get_exchange,
            commands::core::proxy_status,
            commands::core::start_proxy,
            commands::core::stop_proxy,
            commands::core::update_notes,
            commands::search::search_exchanges,
            agent::agent_start,
            agent::agent_confirm_write,
            agent::agent_cancel,
        ])
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
