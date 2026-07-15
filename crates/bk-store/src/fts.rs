//! FTS5 search index. Phase 2.9 lands the real implementation that
//! extracts fields from `HttpExchange` and feeds the contentless
//! `exchange_fts` virtual table. This stub exists so §2.7's
//! `exchanges.rs` compiles and its `insert` test can pass — the FTS
//! sync is intentionally a no-op until §2.9.
//!
//! The signature matches what §2.9 needs: it takes a `&rusqlite::Transaction`
//! (so the FTS write is part of the same atomic unit as the `exchanges`
//! insert) and a `&HttpExchange` to read fields from.

#![allow(missing_docs)]

use crate::error::Result;
use bk_core::HttpExchange;

/// Insert a row in the FTS index for the given exchange. The
/// contentless virtual table expects the same column layout as
/// `schema.rs::MIGRATION_001_INITIAL` declares for `exchange_fts`.
/// §2.9 replaces this no-op with the real call.
pub fn index_exchange(_conn: &rusqlite::Transaction<'_>, _exchange: &HttpExchange) -> Result<()> {
    Ok(())
}
