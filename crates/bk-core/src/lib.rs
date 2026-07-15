//! Core data types for Talon. The whole project shares this crate.

pub mod error;
pub mod ids;

pub use error::{Error, Result};
pub use ids::{ExchangeId, FuzzJobId, Id, NoteId, ProjectId, TagId};
