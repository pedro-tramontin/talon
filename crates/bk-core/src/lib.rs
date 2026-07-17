//! Core data types for Talon. The whole project shares this crate.

pub mod error;
pub mod filter;
pub mod ids;
pub mod model;
pub mod project;
pub mod scope;
pub mod tag;

pub use error::{Error, Result};
pub use filter::{
    Annotation, DecoratedResponse, ExchangeFilter, RequestModifier, ResponseDecorator,
};
pub use ids::{ExchangeId, FuzzJobId, Id, NoteId, ProjectId, TagId};
pub use model::{
    Body, ExchangeMeta, HeaderMap, HttpExchange, Method, Request, Response, ScopeState, Url,
    Version,
};
pub use project::{Project, ProjectInfo, ProjectSettings};
pub use scope::{MatchAction, ScopeRule, ScopeRuleKind};
pub use tag::Tag;
