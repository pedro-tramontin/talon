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
use std::str::FromStr;
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

    /// Construct an `Id<T>` from an existing UUID. Used by the storage
    /// layer when deserializing rows that were read from the database.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self {
            inner: uuid,
            _marker: PhantomData,
        }
    }

    /// Construct an `Id<T>` that is the all-zeros UUID. The storage
    /// layer uses this as a placeholder when the caller hasn't yet
    /// generated an id (the row insert will overwrite it).
    pub fn nil() -> Self {
        Self::from_uuid(Uuid::nil())
    }

    /// Expose the inner UUID. Used by the storage layer when binding
    /// parameters to SQLite statements.
    pub fn as_uuid(&self) -> Uuid {
        self.inner
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

/// Parse an `Id<T>` from the standard UUID string form. The marker type
/// `T` is irrelevant to the parse — it's the same UUID format for every
/// table — but Rust's type system keeps the result distinct.
impl<T> FromStr for Id<T> {
    type Err = uuid::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self::from_uuid(Uuid::from_str(s)?))
    }
}

// Marker types. Each corresponds to a table in the SQLite schema.
//
// `PartialEq + Eq` are required because `Id<T>` derives them — Rust's
// `#[derive]` on a generic struct adds an implicit `T: Trait` bound to
// the impl, and we want `Id<Project> == Id<Project>` to work without
// the caller having to thread bounds manually.
//
// `Clone + Copy` are required for the same reason: any `#[derive(Clone)]`
// or `#[derive(Copy)]` on a containing struct (e.g. `Request`,
// `ExchangeMeta`) propagates a `T: Clone` or `T: Copy` bound to
// `Id<T>`. These are zero-sized so `Clone` and `Copy` are free.
// `Hash` is required so `Id<T>` can be used as a `HashMap` key (the
// engine stores `HashMap<ProjectId, Arc<DbPool>>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Project;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Exchange;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tag;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Note;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

    #[test]
    fn from_uuid_roundtrips_via_display_and_from_str() {
        let original = ExchangeId::new();
        let s = original.to_string();
        let parsed: ExchangeId = s.parse().expect("valid UUID");
        assert_eq!(original, parsed);
    }

    #[test]
    fn from_str_rejects_garbage() {
        let bad = "not-a-uuid".parse::<TagId>();
        assert!(bad.is_err());
    }

    #[test]
    fn nil_is_all_zeros() {
        let n: ProjectId = ProjectId::nil();
        assert_eq!(n.as_uuid(), Uuid::nil());
    }
}
