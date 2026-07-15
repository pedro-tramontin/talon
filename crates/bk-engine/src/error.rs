//! Engine error type. Wraps `bk_core::Error` and `bk_store::StoreError`
//! so the Tauri command bindings (Phase 3+) and the axum handlers
//! (Phase 8) have a single error type to convert into HTTP 500 /
//! Tauri rejection.
//!
//! We rely on `#[derive(thiserror::Error)]` for the `Display` impl.
//! A hand-written `impl fmt::Display` would be a duplicate (and
//! self-recursive, see the note in bk_core::error).

#![allow(missing_docs)]

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("core error: {0}")]
    Core(#[from] bk_core::Error),

    #[error("storage error: {0}")]
    Store(#[from] bk_store::StoreError),

    #[error("project not open: {0}")]
    ProjectNotOpen(String),
}

pub type Result<T> = std::result::Result<T, EngineError>;
