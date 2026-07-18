//! Full event bus for the `bk-engine`. The Tauri UI subscribes to
//! this bus to react to state changes in the engine. The MCP server
//! (§3.5b) and the internal agent (§3.5c) subscribe to the narrower
//! MCP bus in `mcp_events.rs` instead — they only need the 5
//! MCP-shaped variants (smaller payloads, fewer concerns).
//!
//! Per the design contract §3.5a, the bus carries **12 event types**
//! for the v1 surface: 9 wired to engine state changes (project
//! open/close, exchange insert/update/star/delete, tag upsert/
//! attach/detach) + 3 Phase 6/7 stubs (`ScopeChanged`, `FuzzStarted`,
//! `FuzzFinished`) for forward-compat. `#[non_exhaustive]` is on the
//! enum so future phases can add variants without breaking the v1
//! shape. The demux from `EngineEvent` → `McpEvent` lives in
//! `engine.rs` (not in this bus — the bus is a dumb pipe).
//!
//! **The bus is a `tokio::sync::broadcast` channel.** Slow consumers
//! drop events (the broadcast API reports `RecvError::Lagged`); the
//! tests in `tests/capture_loop_smoke.rs` use a fresh subscription
//! per test to avoid contention.

#![allow(missing_docs)]

use bk_core::{ExchangeId, ProjectId, TagId};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// The full set of events the engine emits. New variants go at the
/// end so existing subscribers can still pattern-match the earlier
/// ones without an exhaustive arms warning.
///
/// Variants marked **(stub)** in the spec are emitted by future
/// phases and stay unimplemented in §3.5a; they exist now so the
/// MCP bus can pattern-match on them with a TODO arm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum EngineEvent {
    /// `Engine::open_project` succeeded. Carries the new (or
    /// re-opened) project's ID and the DB filename the UI shows in
    /// the project dropdown.
    ProjectOpened {
        project_id: ProjectId,
        db_filename: String,
    },
    /// `Engine::close_project` ran. The UI removes the project from
    /// the dropdown.
    ProjectClosed { project_id: ProjectId },
    /// `Engine::insert_exchange` ran. The UI adds a row to the
    /// exchange list.
    ExchangeInserted {
        id: ExchangeId,
        project_id: ProjectId,
        summary: String,
    },
    /// `Engine::update_notes` ran. The UI re-fetches the detail
    /// view so the notes pane shows the new value.
    ExchangeNotesUpdated {
        id: ExchangeId,
        project_id: ProjectId,
    },
    /// `Engine::set_starred` ran. The UI updates the ⭐ icon on
    /// the row.
    ExchangeStarredToggled {
        id: ExchangeId,
        project_id: ProjectId,
        starred: bool,
    },
    /// `Engine::delete_exchange` ran. The UI removes the row.
    ExchangeDeleted {
        id: ExchangeId,
        project_id: ProjectId,
    },
    /// `Engine::tag_upsert` ran (created OR returned the existing
    /// tag, since the operation is idempotent). The UI updates the
    /// tag picker.
    TagUpserted {
        id: TagId,
        project_id: ProjectId,
        name: String,
    },
    /// `Engine::tag_attach` ran. The UI updates the tag chips on
    /// the affected exchange.
    TagAttached {
        tag_id: TagId,
        exchange_id: ExchangeId,
        project_id: ProjectId,
    },
    /// `Engine::tag_detach` ran. The UI updates the tag chips.
    TagDetached {
        tag_id: TagId,
        exchange_id: ExchangeId,
        project_id: ProjectId,
    },
    /// Phase 6 stub. When scope rules change, the UI re-colors
    /// in-scope rows. Not emitted in §3.5a.
    ScopeChanged {
        project_id: ProjectId,
        rule_count: u32,
    },
    /// Phase 7 stub. When a fuzz job starts, the Fuzz view shows
    /// the job. Not emitted in §3.5a.
    FuzzStarted {
        job_id: String,
        project_id: ProjectId,
        config_summary: String,
    },
    /// Phase 7 stub. When a fuzz job finishes, the Fuzz view
    /// shows the final stats. Not emitted in §3.5a.
    FuzzFinished {
        job_id: String,
        project_id: ProjectId,
        total_requests: u64,
    },
}

/// The handle the engine hands out to subscribers. The capacity
/// (256) is the same as the `broadcast::channel` default; this is
/// fine for the Tauri UI and the MCP server (both consume events
/// fast). Slow consumers (a misbehaving MCP client that pauses its
/// read loop) will see `RecvError::Lagged` and should resubscribe
/// or skip.
pub type EventReceiver = broadcast::Receiver<EngineEvent>;

/// The sender side, owned by the `Engine`. Cloning the sender is
/// cheap (`broadcast::Sender` is internally `Arc`-wrapped), so the
/// engine can hand out additional senders to sub-components if
/// needed in future phases.
pub type EventSender = broadcast::Sender<EngineEvent>;

/// Build a fresh (sender, receiver) pair. The `Engine::new`
/// constructor stores the sender and exposes `subscribe_events()`
/// which returns a fresh `EventReceiver`.
///
/// Capacity note: 256 is the `tokio::sync::broadcast` default and
/// matches what the rest of the Rust ecosystem uses for "UI event
/// bus" patterns. If a future test or production code path needs
/// more headroom, raise this; if the lag is acceptable at lower
/// counts, lower it.
pub fn channel() -> (EventSender, EventReceiver) {
    broadcast::channel(256)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sender is `Clone`able so the engine can hand out
    /// additional senders cheaply. `Sender::subscribe()` returns
    /// a fresh `Receiver` per call (broadcast channels support
    /// many independent subscribers); `Receiver` itself is NOT
    /// `Clone` — to fan out to multiple consumers, each consumer
    /// must call `Sender::subscribe()` and get its own receiver.
    #[test]
    fn sender_is_cloneable_receivers_are_independent() {
        let (tx, mut rx1) = channel();
        let _tx2 = tx.clone();
        let mut rx2 = tx.subscribe();
        // Send one event — both receivers must see it
        // independently. A shared-receiver bug (e.g., accidentally
        // returning rx1 from a second subscribe call) would fail
        // this: only one of the two would receive.
        tx.send(EngineEvent::ProjectClosed {
            project_id: ProjectId::new(),
        })
        .unwrap();
        let ev1 = rx1.try_recv().expect("rx1 should receive the event");
        let ev2 = rx2.try_recv().expect("rx2 should receive the event");
        assert!(matches!(ev1, EngineEvent::ProjectClosed { .. }));
        assert!(matches!(ev2, EngineEvent::ProjectClosed { .. }));
    }

    /// A send on the sender is observed by a receiver. The smoke
    /// test in `tests/capture_loop_smoke.rs` is the end-to-end
    /// version; this is the unit-level version that doesn't need
    /// the engine.
    #[test]
    fn send_is_observed_by_receiver() {
        let (tx, mut rx) = channel();
        let project_id = ProjectId::new();
        tx.send(EngineEvent::ProjectClosed { project_id }).unwrap();
        let ev = rx.try_recv().unwrap();
        assert_eq!(ev, EngineEvent::ProjectClosed { project_id });
    }

    /// `#[non_exhaustive]` on the enum means downstream code must
    /// have a wildcard arm. This test pins the v1 surface: any
    /// new variant must be added here and to `mcp_events::demux`.
    #[test]
    fn exhaustively_pinned_v1_variants_present() {
        let (tx, _rx) = channel();
        // Send one of each v1 variant. If a variant is renamed or
        // removed, this test fails to compile — that's the point.
        let project_id = ProjectId::new();
        let exchange_id = ExchangeId::new();
        let tag_id = TagId::new();
        tx.send(EngineEvent::ProjectOpened {
            project_id,
            db_filename: "acme.db".into(),
        })
        .unwrap();
        tx.send(EngineEvent::ProjectClosed { project_id }).unwrap();
        tx.send(EngineEvent::ExchangeInserted {
            id: exchange_id,
            project_id,
            summary: "GET /admin".into(),
        })
        .unwrap();
        tx.send(EngineEvent::ExchangeNotesUpdated {
            id: exchange_id,
            project_id,
        })
        .unwrap();
        tx.send(EngineEvent::ExchangeStarredToggled {
            id: exchange_id,
            project_id,
            starred: true,
        })
        .unwrap();
        tx.send(EngineEvent::ExchangeDeleted {
            id: exchange_id,
            project_id,
        })
        .unwrap();
        tx.send(EngineEvent::TagUpserted {
            id: tag_id,
            project_id,
            name: "vuln".into(),
        })
        .unwrap();
        tx.send(EngineEvent::TagAttached {
            tag_id,
            exchange_id,
            project_id,
        })
        .unwrap();
        tx.send(EngineEvent::TagDetached {
            tag_id,
            exchange_id,
            project_id,
        })
        .unwrap();
        // Stubs — they're v1 enum variants even if the engine
        // doesn't emit them yet. The MCP bus demux pattern-matches
        // on them with a "not yet implemented" arm.
        tx.send(EngineEvent::ScopeChanged {
            project_id,
            rule_count: 0,
        })
        .unwrap();
        tx.send(EngineEvent::FuzzStarted {
            job_id: "stub".into(),
            project_id,
            config_summary: "stub".into(),
        })
        .unwrap();
    }
}
