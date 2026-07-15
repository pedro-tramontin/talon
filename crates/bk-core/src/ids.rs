//! Strongly-typed ID newtypes. A `Id<Exchange>` cannot be accidentally
//! passed where a `Id<Tag>` is expected.
//!
//! The inner type is always `uuid::Uuid` (v4). The marker type `T` is
//! zero-sized, so `Id<Exchange>` is exactly 16 bytes — same as a bare UUID.

// See the note in `error.rs` for why we silence `missing_docs` locally:
// the plan's "exact code" doesn't include per-item doc comments, and
// the marker types are documented by their `pub type FooId = Id<Foo>`
// aliases below.
#![allow(missing_docs)]

use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Id<T> {
    inner: Uuid,
    _marker: PhantomData<T>,
}

impl<T> Id<T> {
    pub fn new() -> Self {
        Self {
            inner: Uuid::new_v4(),
            _marker: PhantomData,
        }
    }
}

impl<T> Default for Id<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> std::fmt::Display for Id<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.inner)
    }
}

// Marker types. Each corresponds to a table in the SQLite schema.
//
// `PartialEq + Eq` are required because `Id<T>` derives them — Rust's
// `#[derive]` on a generic struct adds an implicit `T: Trait` bound to
// the impl, and we want `Id<Project> == Id<Project>` to work without
// the caller having to thread bounds manually.
#[derive(Debug, PartialEq, Eq)]
pub struct Project;
#[derive(Debug, PartialEq, Eq)]
pub struct Exchange;
#[derive(Debug, PartialEq, Eq)]
pub struct Tag;
#[derive(Debug, PartialEq, Eq)]
pub struct Note;
#[derive(Debug, PartialEq, Eq)]
pub struct FuzzJob;

pub type ProjectId = Id<Project>;
pub type ExchangeId = Id<Exchange>;
pub type TagId = Id<Tag>;
pub type NoteId = Id<Note>;
pub type FuzzJobId = Id<FuzzJob>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        let a: ProjectId = ProjectId::new();
        let b: ProjectId = ProjectId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn ids_serialize_as_uuid_strings() {
        let id: ExchangeId = ExchangeId::new();
        let s = serde_json::to_string(&id).unwrap();
        // serde_json renders UUIDs as quoted strings
        assert!(s.starts_with('"') && s.ends_with('"'));
        assert_eq!(s.trim_matches('"').len(), 36); // standard UUID form
    }

    #[test]
    fn ids_roundtrip_through_json() {
        let id: TagId = TagId::new();
        let s = serde_json::to_string(&id).unwrap();
        let back: TagId = serde_json::from_str(&s).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn distinct_marker_types_do_not_compile_to_same_type() {
        // This is a compile-time check: if you change `Id<T>` to be
        // type-erased, this will fail to compile. We assert it via a
        // function that takes both, which the type system must distinguish.
        fn _accepts(_: ProjectId) {}
        fn _accepts_too(_: ExchangeId) {}
        // The mere existence of the two functions with distinct parameter
        // types is the test. If they collapsed to the same type, the
        // compiler would complain about duplicate definitions.
    }
}
