//! Public engine API for Talon. This is the crate the Tauri shell and the
//! axum browser-server call into.
//!
//! Phase 2 lands the project open/close API. Later phases add replay (5),
//! match & replace + scope (6), and fuzzer (7).

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
