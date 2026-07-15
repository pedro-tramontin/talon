//! Talon core types.
//!
//! `bk-core` holds the shared data model used by every other crate. It has
//! no I/O, no async runtime, and no platform-specific code — it's a pure
//! data + trait crate so it stays cheap to compile and trivial to test.
//!
//! Phase 1 contains only the bare minimum (an identifier type and a
//! version constant) to prove the workspace is wired up. The HTTP
//! exchange, request/response, and project types arrive in Phase 2.

#![deny(unsafe_code)]
#![warn(missing_docs)]

use serde::{Deserialize, Serialize};

/// Talon's build-time version. From `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A globally-unique identifier for a Talon project (one project = one
/// `.db` file). Generated once at project creation time and never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectId(pub uuid::Uuid);

impl ProjectId {
    /// Create a new random project ID.
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for ProjectId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ProjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "proj_{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The version string must be non-empty and look like a semver tag.
    /// We don't pin it in the test (the build script will produce different
    /// values in dev vs release) — we just assert it's well-formed.
    #[test]
    fn version_is_well_formed_semver() {
        let v = VERSION;
        assert!(!v.is_empty(), "VERSION must not be empty");
        let parts: Vec<&str> = v.split('.').collect();
        assert!(
            parts.len() >= 2 && parts.len() <= 3,
            "expected semver with 2-3 dot-separated parts, got {v:?}"
        );
        for p in &parts[..2] {
            assert!(
                p.chars().all(|c| c.is_ascii_digit()),
                "expected numeric major.minor, got {p:?} in {v:?}"
            );
        }
    }

    /// Two consecutively-generated IDs must differ. Catches a UUID-PRNG
    /// regression where every call returns the same value.
    #[test]
    fn new_ids_are_unique() {
        let a = ProjectId::new();
        let b = ProjectId::new();
        assert_ne!(a, b, "two consecutive ProjectId::new() must not collide");
    }

    /// The Display impl must produce a stable, parseable, human-readable form.
    /// We don't parse it back (that would couple the test to a specific format
    /// we may want to change), but we assert it has the `proj_` prefix and
    /// includes the UUID.
    #[test]
    fn display_includes_prefix_and_uuid() {
        let id = ProjectId::new();
        let s = format!("{id}");
        assert!(s.starts_with("proj_"), "expected proj_ prefix, got {s:?}");
        let suffix = &s[5..];
        assert_eq!(
            suffix.len(),
            36,
            "expected 36-char UUID suffix, got {suffix:?} (len {})",
            suffix.len()
        );
    }
}
