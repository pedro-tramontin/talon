//! Tag type. A `Tag` is a named label scoped to a project that can
//! be attached to one or more exchanges. Tags are reusable: the same
//! tag can be attached to many exchanges, and many tags can attach
//! to the same exchange.
//!
//! The storage layer (`bk_store::tags`) manages the per-project tag
//! rows and the `exchange_tags` join table. This module just defines
//! the type so the engine can hand it across the FFI boundary to
//! the UI without leaking storage internals.
//!
//! The pre-defined tags (🔴 vuln, 🔑 creds, 📌 interesting, ⏭ ignore)
//! are just regular tag rows seeded by the UI on first run — they
//! have no special meaning in the storage layer. The `starred` flag
//! on `exchanges` is intentionally separate from the tags system
//! (see `ExchangeMeta`).

#![allow(missing_docs)]

use serde::{Deserialize, Serialize};

/// A named label that can be attached to exchanges within a project.
/// The `(project_id, name)` pair is unique in the storage layer
/// (the `tags` table has a `UNIQUE (project_id, name)` constraint).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub id: crate::TagId,
    pub name: String,
    /// Optional CSS color (e.g. "#ef4444"). When `None`, the UI
    /// picks a default from a hash of the tag name.
    pub color: Option<String>,
}
