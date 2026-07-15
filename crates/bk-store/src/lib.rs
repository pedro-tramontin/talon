//! SQLite persistence layer for Talon projects.
//!
//! Phase 2 lands the schema, migrations, exchanges CRUD, tags, notes, and
//! FTS5 search. The HTTP proxy (Phase 3) and fuzzer (Phase 7) will both
//! write to this store.

pub mod db;
pub mod error;
pub mod exchanges;
pub mod fts;
pub mod migrations;
pub mod schema;

pub use db::{open, project_path, DbPool};
pub use error::{Result, StoreError};
