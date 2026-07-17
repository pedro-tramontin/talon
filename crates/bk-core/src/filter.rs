//! Extension-point traits for the v2 plugin system.
//!
//! **This file exists in v1 as a no-op placeholder** so v2's
//! `bk-plugin` crate can implement these traits without
//! refactoring the Engine or the proxy pipeline. Per the Phase 10
//! plugin-system design contract (§5.1 item 4 + §5.2 items 1, 4, 6):
//!
//! - The traits below are **the contract surface** that v1
//!   callers (Engine, proxy pipeline, fuzzer runner) use.
//! - v1 ships **zero implementations** of any of these traits.
//! - v1 callers do `if let Some(reg) = &self.plugins { ... }` so
//!   the v2 registry slot doesn't impose a v1 cost.
//! - **No `bk-plugin` crate, no `wasmtime` dep, no manifest
//!   type, no host functions** — those land in v2.
//!
//! # Why a placeholder file at all?
//!
//! If we don't define these traits in v1, the v2 implementation
//! has to:
//!
//! 1. Add the traits here.
//! 2. Refactor `bk-engine::Engine` to hold an
//!    `Option<Arc<PluginRegistry>>`.
//! 3. Refactor `bk-proxy::pipeline` to call `apply_filters`.
//! 4. Refactor every v1 caller to handle the new field / method.
//!
//! With the placeholder: v2 is just "fill in the `bk-plugin`
//! crate, set the Engine's `plugins: None` to `Some(reg)`."
//! That's a 1-day v2 PR instead of a 2-week v2 refactor.
//!
//! # Why trait objects (`dyn ...`) and not generics?
//!
//! The v2 plugin registry stores `Vec<Arc<dyn ExchangeFilter>>`
//! etc. — heterogeneous plugin types, dynamic registration,
//! loaded/unloaded at runtime. Generics don't compose across
//! runtime-loaded types. `Box<dyn ...>` is the only choice that
//! doesn't make plugin loading a compile-time problem.
//!
//! # Why `Send + Sync`?
//!
//! The Engine is shared across the tokio runtime (the proxy
//! listener, the fuzzer runner, the axum server, the MCP bus).
//! Anything stored in an `Arc` and shared must be `Send + Sync`
//! to cross tasks. The trait bounds enforce this at the type
//! level; v2 implementations that accidentally capture a
//! `Rc<...>` or a non-thread-safe handle will fail to compile.
// See the note in `error.rs` for why we silence `missing_docs` locally:
// the plan's "exact code" doesn't include per-item doc comments, and the
// field-level docs are covered by the struct-level docs.
#![allow(missing_docs)]

use crate::{HttpExchange, Request, Response};
use serde::Serialize;

/// A predicate that decides whether an exchange is "interesting."
///
/// v1 has zero implementations. v2 plugins (e.g. "show me all
/// requests that contain a JWT in the Authorization header where
/// the JWT's `alg` field is `none`") implement this trait and
/// register the implementation in the `PluginRegistry`.
///
/// **Wire format for v2 filter results:** `true` means the
/// exchange passes the filter (the proxy pipeline keeps it,
/// the UI's filter row shows it, etc.); `false` means the
/// exchange is filtered out.
///
/// **Performance budget:** `matches()` is called once per
/// exchange on the proxy hot path. Implementations must be
/// cheap — no I/O, no `block_in_place`, no sleeps. If a
/// plugin needs more than ~10µs per call, it should batch
/// in the background instead.
pub trait ExchangeFilter: Send + Sync {
    /// Human-readable name shown in the UI's filter list
    /// (e.g. "JWT alg=none", "GraphQL mutations", "5xx only").
    fn name(&self) -> &str;

    /// Evaluate. Returning `true` keeps the exchange; `false`
    /// filters it out of the default view.
    fn matches(&self, exchange: &HttpExchange) -> bool;
}

/// Modifies a request before it goes upstream.
///
/// v1 has zero implementations (the M&R engine in Phase 6 is a
/// separate, faster, non-trait path because it runs on every
/// byte of every request — a trait dispatch per byte would be
/// too slow). v2 plugins that want to do "rewrite Authorization
/// header to use a custom token format" or "inject a tracing
/// header with a per-project UUID" implement this trait.
///
/// **Ordering:** multiple modifiers compose in registration
/// order. The v2 registry sorts by `(priority DESC, registered
/// ASC)` so a plugin's modifier can declare it should run before
/// or after the built-in M&R.
///
/// **Failure mode:** if `modify()` returns `Err`, the proxy
/// pipeline MUST fall back to the original request (not the
/// modified one) and emit a `ProxyEvent::PluginModifierFailed`
/// so the UI can warn the user. A buggy plugin must not
/// corrupt the request.
pub trait RequestModifier: Send + Sync {
    /// Human-readable name shown in the UI's modifier list
    /// (e.g. "Add X-Talon-Trace-Id", "Rewrite JWT alg").
    fn name(&self) -> &str;

    /// Mutate the request in place. Return `Ok(())` on success
    /// or `Err(e)` on failure; see the failure-mode note above
    /// for what the proxy does on `Err`.
    fn modify(&self, req: &mut Request) -> Result<(), crate::Error>;
}

/// Decorates a response for display.
///
/// v1 has zero implementations. v2 plugins that want to do
/// "auto-decrypt this proprietary protocol" or "highlight the
/// SQL error in red" implement this trait and produce a
/// `DecoratedResponse` for the UI's right rail to render.
///
/// **No side effects:** this is a pure-function transformation
/// for display only. The original `Response` in storage is
/// never mutated; the decoration is computed on read.
pub trait ResponseDecorator: Send + Sync {
    /// Human-readable name shown in the UI's decorator list
    /// (e.g. "Auto-decrypt AES-CBC", "Highlight SQL errors").
    fn name(&self) -> &str;

    /// Compute the decoration. The implementation may do
    /// expensive work (e.g. attempt a decryption); the UI
    /// shows a "decorating..." spinner while it runs.
    fn decorate(&self, resp: &Response) -> DecoratedResponse;
}

/// The output of a `ResponseDecorator`.
///
/// `annotations` are UI-side highlights (line/column ranges
/// with a kind tag like `"sql_error"` or `"jwt_invalid"`).
/// `transformed_body` is an optional alternative body the UI
/// can show in a "decorated" tab — used by the auto-decrypt
/// case where the user wants to see the plaintext, not the
/// ciphertext.
#[derive(Debug, Clone, Serialize)]
pub struct DecoratedResponse {
    /// Human-readable label (e.g. "Decrypted with key X",
    /// "3 SQL errors highlighted").
    pub label: String,
    /// UI highlight regions. Each entry is a (line, col, kind)
    /// triple; the UI renders them as colored underlines.
    pub annotations: Vec<Annotation>,
    /// Optional alternative body (e.g. decrypted text). If
    /// `None`, the UI falls back to the original body.
    pub transformed_body: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Annotation {
    /// 0-based line number in the rendered body.
    pub line: u32,
    /// 0-based column number in the line.
    pub col: u32,
    /// Length of the highlighted region in characters.
    pub length: u32,
    /// UI-side tag for the highlight color. Convention:
    /// `"sql_error"` = red, `"jwt_invalid"` = orange,
    /// `"info"` = blue, etc. v2 plugins can introduce new
    /// tags; the UI maps unknown tags to a default color.
    pub kind: String,
}

#[cfg(test)]
mod tests {
    //! v1 ships zero implementations, so these tests only
    //! assert the trait shapes compile. v2 will add tests
    //! that exercise real plugin behavior.

    use super::*;

    /// A trivial ExchangeFilter impl, used to verify the trait
    /// shape compiles. Not registered anywhere in v1.
    struct AlwaysTrue;
    impl ExchangeFilter for AlwaysTrue {
        fn name(&self) -> &str {
            "always-true"
        }
        fn matches(&self, _exchange: &HttpExchange) -> bool {
            true
        }
    }

    #[test]
    fn trait_object_works_for_exchange_filter() {
        // The whole point of v1 having this file: prove that
        // `Box<dyn ExchangeFilter>` is the shape v2 will use.
        let f: Box<dyn ExchangeFilter> = Box::new(AlwaysTrue);
        assert_eq!(f.name(), "always-true");
    }

    /// A trivial RequestModifier impl, same purpose.
    struct NoopModifier;
    impl RequestModifier for NoopModifier {
        fn name(&self) -> &str {
            "noop"
        }
        fn modify(&self, _req: &mut Request) -> Result<(), crate::Error> {
            Ok(())
        }
    }

    #[test]
    fn trait_object_works_for_request_modifier() {
        let m: Box<dyn RequestModifier> = Box::new(NoopModifier);
        assert_eq!(m.name(), "noop");
    }

    /// A trivial ResponseDecorator impl, same purpose.
    struct NoopDecorator;
    impl ResponseDecorator for NoopDecorator {
        fn name(&self) -> &str {
            "noop"
        }
        fn decorate(&self, _resp: &Response) -> DecoratedResponse {
            DecoratedResponse {
                label: "noop".into(),
                annotations: vec![],
                transformed_body: None,
            }
        }
    }

    #[test]
    fn trait_object_works_for_response_decorator() {
        let d: Box<dyn ResponseDecorator> = Box::new(NoopDecorator);
        assert_eq!(d.name(), "noop");
    }
}
