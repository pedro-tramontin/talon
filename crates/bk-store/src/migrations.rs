//! Schema migrations. Stub for §2.5 — the real migration logic lands in
//! §2.6 (Phase 2 Part B). This stub exists so `db::open()` can compile and
//! the §2.5 tests can pass; it does nothing.

#![allow(missing_docs)]

use crate::error::Result;
use rusqlite::Connection;

/// Apply all pending migrations to the given connection.
///
/// In §2.5 this is a no-op. §2.6 replaces it with the real
/// `CREATE TABLE` + version-tracking logic.
pub fn run(_conn: &Connection) -> Result<()> {
    Ok(())
}
