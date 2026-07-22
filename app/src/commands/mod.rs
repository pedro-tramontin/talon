//! Tauri command surface for the engine + proxy wiring.
//!
//! Sub-modules:
//! - [`core`]: §4.1 + §4.7 commands (`open_project`,
//!   `close_project`, `list_exchanges`, `get_exchange`,
//!   `proxy_status`, `start_proxy`, `stop_proxy`,
//!   `update_notes`).
//! - [`search`]: §4.8 `search_exchanges` (FTS5 wrapper).
//! - [`replay`]: Phase 5 §5.1 + §5.2 commands
//!   (`open_replay_tab`, `send_replay`).
//!
//! The Tauri `generate_handler!` macro in `app/src/lib.rs`
//! references each command by its module path (e.g.
//! `commands::core::open_project`,
//! `commands::search::search_exchanges`,
//! `commands::replay::open_replay_tab`); see that file
//! for the canonical registration list.

pub mod core;
pub mod replay;
pub mod scope;
pub mod search;

pub use core::EngineArc;
