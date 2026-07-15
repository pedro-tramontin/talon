//! SQLite persistence layer for Talon projects.
//!
//! Phase 2 lands the schema, migrations, exchanges CRUD, tags, notes, and
//! FTS5 search. The HTTP proxy (Phase 3) and fuzzer (Phase 7) will both
//! write to this store.

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_semver() {
        assert!(version().contains('.'));
    }
}
