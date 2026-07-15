//! bk-store error type. Implements `From<StoreError> for bk_core::Error`
//! so the engine can return a single error type upward.
//!
//! Note: we deliberately do NOT add a manual `impl fmt::Display for StoreError`
//! here. `#[derive(thiserror::Error)]` already provides `Display`, and a
//! second manual impl would be a duplicate-impl conflict. (The bk-core §2.2
//! plan had the same footgun; same fix here.)

#![allow(missing_docs)]

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("connection pool error: {0}")]
    Pool(#[from] r2d2::Error),

    #[error("migration {0} failed: {1}")]
    Migration(u32, String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid input: {0}")]
    Invalid(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<StoreError> for bk_core::Error {
    fn from(e: StoreError) -> Self {
        bk_core::Error::Storage(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, StoreError>;
