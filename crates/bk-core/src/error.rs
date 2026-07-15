//! Canonical error type. Other crates convert their own errors into this
//! using `From` impls, so the public API surface stays uniform.

// Doc comments on every variant/type-alias would be noise here — the
// `#[error("...")]` attribute already documents each variant's meaning,
// and the module-level doc comment covers the type alias. The plan's
// "exact code" does not include per-item docs, so we silence the
// crate-level `missing_docs` warning locally rather than rewrite the
// plan's code.
#![allow(missing_docs)]

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("project not found: {0}")]
    ProjectNotFound(String),

    #[error("exchange not found: {0}")]
    ExchangeNotFound(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid input: {0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;

// NOTE: The Phase 2 plan included a manual `impl fmt::Display for Error`
// block here. It has been removed because:
//   1. `#[derive(thiserror::Error)]` already generates a Display impl,
//      so the manual block was a duplicate (would not compile).
//   2. The manual block in the plan was self-recursive
//      (`write!(f, "{}", self)` where `self: &Error`), which would
//      have caused infinite recursion / stack overflow at runtime.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_not_found_displays_name() {
        let e = Error::ProjectNotFound("acme.bb".to_string());
        assert_eq!(e.to_string(), "project not found: acme.bb");
    }

    #[test]
    fn from_io_error() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "nope");
        let e: Error = io.into();
        assert!(matches!(e, Error::Io(_)));
    }

    #[test]
    fn from_serde_error() {
        let bad = serde_json::from_str::<i32>("not a number");
        let e: Error = bad.unwrap_err().into();
        assert!(matches!(e, Error::Serde(_)));
    }
}
