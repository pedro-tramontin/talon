//! Tauri command surface for the engine + proxy wiring.
//!
//! Sub-modules:
//! - [`core`]: §4.1 + §4.7 commands (`open_project`,
//!   `close_project`, `list_exchanges`, `get_exchange`,
//!   `proxy_status`, `start_proxy`, `stop_proxy`,
//!   `update_notes`).
//! - [`search`]: §4.8 `search_exchanges` (FTS5 wrapper).
//!
//! The Tauri `generate_handler!` macro in `app/src/lib.rs`
//! references each command by its module path (e.g.
//! `commands::core::open_project`,
//! `commands::search::search_exchanges`); see that file
//! for the canonical registration list.

pub mod core;
pub mod search;

pub use core::EngineArc;
